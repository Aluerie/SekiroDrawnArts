//! Drawn Arts, a Sekiro mod loaded by Mod Engine 3: the selected combat art is equipped only
//! while a key is held, and a wheel picks which art that is.

mod arts;
mod camera;
mod config;
mod controller;
mod game;
mod icons;
mod input;
mod logger;
mod wheel;

use std::{
    ffi::c_void,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, anyhow};
use hudhook::{
    Hudhook,
    hooks::dx11::ImguiDx11Hooks,
    mh::{MH_ApplyQueued, MhHook},
};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HMODULE},
        System::LibraryLoader::{
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
            GetModuleHandleExW,
        },
    },
    core::PCWSTR,
};

use crate::{config::Config, controller::Controller, game::ProcessInputFn, wheel::SharedWheel};

const CONFIG_FILE: &str = "drawn_arts.ini";
const LOG_FILE: &str = "drawn_arts.log";
const ICON_DIR: &str = "icons";

static CONTROLLER: OnceLock<Mutex<Controller>> = OnceLock::new();
static ORIGINAL_PROCESS_INPUT: OnceLock<ProcessInputFn> = OnceLock::new();

/// Installs the mod. Mod Engine 3 calls this once the game is running, as named by the
/// `initializer` of this DLL's entry in the `.me3` profile.
///
/// Returns whether the mod was installed; details of a failure go to `drawn_arts.log`.
#[unsafe(no_mangle)]
pub extern "C" fn drawn_arts_init() -> bool {
    let result = panic::catch_unwind(|| {
        let (module, dir) = this_module().context("can't locate drawn_arts.dll")?;
        logger::init(&dir.join(LOG_FILE));
        log::info!("Drawn Arts {} starting from {}", env!("CARGO_PKG_VERSION"), dir.display());
        install(module, &dir)
    });
    match result {
        Ok(Ok(())) => {
            log::info!("ready: hold the hold key to equip, hold the wheel key to choose");
            true
        }
        Ok(Err(e)) => {
            log::error!("failed to set up the mod: {e:#}");
            false
        }
        Err(_) => {
            log::error!("the mod panicked while setting up");
            false
        }
    }
}

/// Returns this DLL's module handle and the directory it was loaded from.
fn this_module() -> anyhow::Result<(HMODULE, PathBuf)> {
    let mut module = HMODULE::default();
    let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    let address = drawn_arts_init as *const c_void;
    unsafe { GetModuleHandleExW(flags, PCWSTR(address.cast()), &mut module) }?;

    let mut buf = [0u16; 1024];
    let len = unsafe { GetModuleFileNameW(Some(module), &mut buf) } as usize;
    let path = PathBuf::from(String::from_utf16_lossy(&buf[..len]));
    let dir = path.parent().map(Path::to_path_buf).context("DLL path has no parent directory")?;
    Ok((module, dir))
}

/// Hooks the game's input processing and draws the wheel overlay.
fn install(module: HMODULE, dir: &Path) -> anyhow::Result<()> {
    let timestamp = game::check_executable().context("this isn't the Steam version 1.06 of Sekiro")?;
    log::info!("sekiro.exe build timestamp: {timestamp:#010x}");
    let config = Config::load(&dir.join(CONFIG_FILE))?;
    log::set_max_level(config.log_level);
    log::info!("settings: {config:?}");
    let wheel = SharedWheel::default();
    CONTROLLER.set(Mutex::new(Controller::new(config, wheel.clone()))).map_err(|_| anyhow!("mod installed twice"))?;

    // Building hudhook initializes MinHook, which the input hook below relies on.
    let overlay = Hudhook::builder()
        .with::<ImguiDx11Hooks>(wheel::Overlay::new(wheel, dir.join(ICON_DIR)))
        .with_hmodule(HINSTANCE(module.0))
        .build();

    let hook = unsafe { MhHook::new(game::PROCESS_INPUT as *mut c_void, process_input as *mut c_void) }
        .map_err(|status| anyhow!("can't hook input processing: {status:?}"))?;
    let original: ProcessInputFn = unsafe { std::mem::transmute(hook.trampoline()) };
    ORIGINAL_PROCESS_INPUT.set(original).map_err(|_| anyhow!("input hook installed twice"))?;
    unsafe { hook.queue_enable() }.map_err(|status| anyhow!("can't enable the input hook: {status:?}"))?;
    // The wheel still works without these hooks; the camera just keeps turning while aiming.
    let camera_hooked =
        camera::queue_hooks(module).inspect_err(|e| log::warn!("can't freeze the camera: {e:#}")).is_ok();
    unsafe { MH_ApplyQueued() }.ok().map_err(|status| anyhow!("can't apply the input hooks: {status:?}"))?;
    log::info!("hooked the game's input processing at {:#x}", game::PROCESS_INPUT);
    if camera_hooked {
        log::info!("hooked DirectInput's mouse to freeze the camera while the wheel is open");
    }

    // The hold key works without the overlay; only the wheel is lost.
    match overlay.apply() {
        Ok(()) => log::info!("hooked DirectX 11 for the wheel overlay"),
        Err(status) => log::warn!("can't hook DirectX 11, so the wheel won't be drawn: {status:?}"),
    }
    Ok(())
}

/// Replacement for the game's per-frame input processing that runs the mod first.
unsafe extern "system" fn process_input(handler: *mut game::InputHandler, arg: usize) -> usize {
    static CALLED: AtomicBool = AtomicBool::new(false);
    static SAVE_LOADED: AtomicBool = AtomicBool::new(false);

    if !CALLED.swap(true, Ordering::Relaxed) {
        log::info!("the game called the input hook for the first time");
    }
    if let (Some(handler), Some(controller)) = (unsafe { handler.as_mut() }, CONTROLLER.get()) {
        // SAFETY: this hook runs on the game's main thread.
        let player = unsafe { game::Player::current() };
        let was_loaded = SAVE_LOADED.swap(player.is_some(), Ordering::Relaxed);
        if was_loaded && player.is_none() {
            log::info!("save unloaded");
        }
        if let Some(player) = player {
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                let mut controller = controller.lock().unwrap_or_else(PoisonError::into_inner);
                if !was_loaded {
                    controller.on_save_loaded(&player);
                }
                controller.tick(&player, &mut handler.action);
            }));
            if result.is_err() {
                log::error!("the mod panicked while processing input");
            }
        }
    }
    let original = ORIGINAL_PROCESS_INPUT.get().expect("hook enabled before the trampoline was stored");
    unsafe { original(handler, arg) }
}
