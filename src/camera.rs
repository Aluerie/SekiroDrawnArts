//! Keeps the game's camera still while the wheel is open by hiding mouse movement from the game.
//!
//! The game reads the mouse through DirectInput, so the two device methods that report mouse
//! input are hooked: while the camera is frozen, they report no movement along X and Y, and the
//! movement they hide is collected for steering the wheel.

use std::{
    ffi::c_void,
    mem::{offset_of, size_of},
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
};

use anyhow::{Context, anyhow};
use hudhook::mh::MhHook;
use windows::{
    Win32::{
        Devices::HumanInterfaceDevice::{
            DI8DEVTYPE_MOUSE, DIDEVCAPS, DIDEVICEOBJECTDATA, DIGDD_PEEK, DIMOUSESTATE, DIMOUSESTATE2,
            DIRECTINPUT_VERSION, DirectInput8Create, GUID_SysMouse, IDirectInput8A, IDirectInput8W,
        },
        Foundation::{HINSTANCE, HMODULE},
    },
    core::{HRESULT, IUnknown, Interface},
};

/// Positions of the hooked methods in the `IDirectInputDevice8` vtable.
const GET_CAPABILITIES: usize = 3;
const GET_DEVICE_STATE: usize = 9;
const GET_DEVICE_DATA: usize = 10;

type GetCapabilitiesFn = unsafe extern "system" fn(*mut c_void, *mut DIDEVCAPS) -> HRESULT;
type GetDeviceStateFn = unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT;
type GetDeviceDataFn = unsafe extern "system" fn(*mut c_void, u32, *mut DIDEVICEOBJECTDATA, *mut u32, u32) -> HRESULT;

static FROZEN: AtomicBool = AtomicBool::new(false);
/// Whether the game has read the mouse through the hooks at least once.
static HOOKED_MOUSE: AtomicBool = AtomicBool::new(false);
/// Mouse movement hidden from the game since the last [`take_motion`].
static MOTION_X: AtomicI32 = AtomicI32::new(0);
static MOTION_Y: AtomicI32 = AtomicI32::new(0);
static ORIGINAL_GET_DEVICE_STATE: OnceLock<Vec<(usize, GetDeviceStateFn)>> = OnceLock::new();
static ORIGINAL_GET_DEVICE_DATA: OnceLock<Vec<(usize, GetDeviceDataFn)>> = OnceLock::new();

/// Stops or resumes the game's camera following the mouse.
pub fn freeze(frozen: bool) {
    if frozen {
        MOTION_X.store(0, Ordering::Relaxed);
        MOTION_Y.store(0, Ordering::Relaxed);
    }
    FROZEN.store(frozen, Ordering::Relaxed);
}

/// Whether the game reads the mouse through the hooks, so freezing works and [`take_motion`]
/// reports movement.
pub fn is_active() -> bool {
    HOOKED_MOUSE.load(Ordering::Relaxed)
}

/// Returns the raw mouse movement hidden from the game since the last call, in mouse counts, if
/// there was any. Only movement made while the camera is frozen is collected.
pub fn take_motion() -> Option<(f32, f32)> {
    let dx = MOTION_X.swap(0, Ordering::Relaxed);
    let dy = MOTION_Y.swap(0, Ordering::Relaxed);
    (dx != 0 || dy != 0).then_some((dx as f32, dy as f32))
}

/// Creates the DirectInput hooks and queues them for enabling with `MH_ApplyQueued`.
///
/// MinHook must already be initialized.
pub fn queue_hooks(module: HMODULE) -> anyhow::Result<()> {
    // The ANSI and Unicode devices have separate vtables, and the game may use either.
    let mut hooks = Vec::new();
    let mut state = Vec::new();
    let mut data = Vec::new();
    for vtable in [mouse_vtable::<IDirectInput8W>(module)?, mouse_vtable::<IDirectInput8A>(module)?] {
        let (state_target, data_target) = unsafe { (*vtable.add(GET_DEVICE_STATE), *vtable.add(GET_DEVICE_DATA)) };
        if !state.iter().any(|&(target, _)| target == state_target) {
            let hook = unsafe { create_hook(state_target, get_device_state as *mut c_void) }?;
            state.push((state_target, unsafe {
                std::mem::transmute::<*mut c_void, GetDeviceStateFn>(hook.trampoline())
            }));
            hooks.push(hook);
        }
        if !data.iter().any(|&(target, _)| target == data_target) {
            let hook = unsafe { create_hook(data_target, get_device_data as *mut c_void) }?;
            data.push((data_target, unsafe { std::mem::transmute::<*mut c_void, GetDeviceDataFn>(hook.trampoline()) }));
            hooks.push(hook);
        }
    }
    // The detours need the trampolines, so nothing is enabled before they're stored.
    ORIGINAL_GET_DEVICE_STATE.set(state).map_err(|_| anyhow!("mouse hooks installed twice"))?;
    ORIGINAL_GET_DEVICE_DATA.set(data).map_err(|_| anyhow!("mouse hooks installed twice"))?;
    for hook in &hooks {
        unsafe { hook.queue_enable() }.map_err(|status| anyhow!("can't enable a mouse hook: {status:?}"))?;
    }
    Ok(())
}

unsafe fn create_hook(target: usize, detour: *mut c_void) -> anyhow::Result<MhHook> {
    unsafe { MhHook::new(target as *mut c_void, detour) }
        .map_err(|status| anyhow!("can't hook DirectInput: {status:?}"))
}

/// Creates a throwaway mouse device through the DirectInput interface `I` and returns its vtable,
/// which every mouse device of that interface shares.
fn mouse_vtable<I: Interface>(module: HMODULE) -> anyhow::Result<*const usize> {
    let mut raw = ptr::null_mut();
    unsafe { DirectInput8Create(HINSTANCE(module.0), DIRECTINPUT_VERSION, &I::IID, &mut raw, None::<&IUnknown>) }
        .context("can't create DirectInput")?;
    // SAFETY: DirectInput8Create returned an owned reference to an `I`.
    let input = unsafe { I::from_raw(raw) };
    // Both interface versions put CreateDevice right after the IUnknown methods.
    type CreateDeviceFn =
        unsafe extern "system" fn(*mut c_void, *const windows::core::GUID, *mut *mut c_void, *mut c_void) -> HRESULT;
    let create_device: CreateDeviceFn = unsafe { std::mem::transmute(*vtable_of(input.as_raw()).add(3)) };
    let mut device = ptr::null_mut();
    unsafe { create_device(input.as_raw(), &GUID_SysMouse, &mut device, ptr::null_mut()) }
        .ok()
        .context("can't create a DirectInput mouse")?;
    // SAFETY: CreateDevice returned an owned device reference, released when this drops.
    let device = unsafe { IUnknown::from_raw(device) };
    Ok(vtable_of(device.as_raw()))
}

fn vtable_of(object: *mut c_void) -> *const usize {
    unsafe { *object.cast::<*const usize>() }
}

fn original<F: Copy>(originals: &OnceLock<Vec<(usize, F)>>, device: *mut c_void, index: usize) -> F {
    let target = unsafe { *vtable_of(device).add(index) };
    let originals = originals.get().expect("hook enabled before its trampoline was stored");
    originals.iter().find(|&&(t, _)| t == target).unwrap_or(&originals[0]).1
}

/// Logs once that mouse input passes through the hooks, to help diagnose a camera that still moves.
fn report_hooked() {
    if !HOOKED_MOUSE.swap(true, Ordering::Relaxed) {
        log::info!("the game reads the mouse through DirectInput; the camera can be frozen");
    }
}

fn is_mouse(device: *mut c_void) -> bool {
    let get_capabilities: GetCapabilitiesFn = unsafe { std::mem::transmute(*vtable_of(device).add(GET_CAPABILITIES)) };
    let mut caps = DIDEVCAPS { dwSize: size_of::<DIDEVCAPS>() as u32, ..Default::default() };
    unsafe { get_capabilities(device, &mut caps) }.is_ok() && caps.dwDevType & 0xFF == DI8DEVTYPE_MOUSE
}

/// Replacement for `IDirectInputDevice8::GetDeviceState`, which reads a device's current state.
unsafe extern "system" fn get_device_state(device: *mut c_void, size: u32, data: *mut c_void) -> HRESULT {
    let result = unsafe { original(&ORIGINAL_GET_DEVICE_STATE, device, GET_DEVICE_STATE)(device, size, data) };
    let mouse_state = [size_of::<DIMOUSESTATE>(), size_of::<DIMOUSESTATE2>()].contains(&(size as usize));
    if result.is_ok() && mouse_state && !data.is_null() {
        report_hooked();
        if FROZEN.load(Ordering::Relaxed) {
            // Both mouse state layouts start with the X and Y movement.
            let state = unsafe { &mut *data.cast::<DIMOUSESTATE>() };
            MOTION_X.fetch_add(std::mem::take(&mut state.lX), Ordering::Relaxed);
            MOTION_Y.fetch_add(std::mem::take(&mut state.lY), Ordering::Relaxed);
        }
    }
    result
}

/// Replacement for `IDirectInputDevice8::GetDeviceData`, which reads a device's buffered events.
unsafe extern "system" fn get_device_data(
    device: *mut c_void,
    object_size: u32,
    events: *mut DIDEVICEOBJECTDATA,
    count: *mut u32,
    flags: u32,
) -> HRESULT {
    let result = unsafe {
        original(&ORIGINAL_GET_DEVICE_DATA, device, GET_DEVICE_DATA)(device, object_size, events, count, flags)
    };
    if result.is_err() || events.is_null() || count.is_null() || !FROZEN.load(Ordering::Relaxed) || !is_mouse(device) {
        return result;
    }
    report_hooked();
    let x_axis = offset_of!(DIMOUSESTATE, lX) as u32;
    let y_axis = offset_of!(DIMOUSESTATE, lY) as u32;
    // A peek leaves the events in the buffer for the next read, which would count them again.
    let collect = flags & DIGDD_PEEK == 0;
    let stride = object_size as usize;
    let base = events.cast::<u8>();
    let mut kept = 0;
    // SAFETY: the game's buffer holds `*count` events of `object_size` bytes, each starting with
    // the `DIDEVICEOBJECTDATA` fields: the offset of the axis or button, then its value.
    unsafe {
        for i in 0..*count as usize {
            let event = base.add(i * stride);
            let axis = event.cast::<u32>().read_unaligned();
            // Relative axis values are signed movements stored in an unsigned field.
            let value = event.add(offset_of!(DIDEVICEOBJECTDATA, dwData)).cast::<i32>().read_unaligned();
            if axis == x_axis || axis == y_axis {
                if collect {
                    let motion = if axis == x_axis { &MOTION_X } else { &MOTION_Y };
                    motion.fetch_add(value, Ordering::Relaxed);
                }
                continue;
            }
            if kept != i {
                ptr::copy(event, base.add(kept * stride), stride);
            }
            kept += 1;
        }
        *count = kept as u32;
    }
    result
}
