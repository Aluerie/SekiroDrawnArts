//! Addresses, memory layouts and functions of `sekiro.exe` version 1.06.
//!
//! Every address here is absolute: the game is not built with ASLR, so its image always
//! loads at `0x140000000`. Other game versions have different addresses and are not supported.

use std::{ffi::c_void, fmt, marker::PhantomData, mem::offset_of, num::NonZeroU32, ops::BitOr, ptr::NonNull};

use anyhow::bail;
use windows::{Win32::System::LibraryLoader::GetModuleHandleW, core::PCWSTR};

/// Address the game's executable image always loads at.
const IMAGE_BASE: usize = 0x1_4000_0000;

/// Address of the function that turns the frame's raw input into the player's action flags.
pub const PROCESS_INPUT: usize = 0x1_40B2_C190;
/// Address of the function that maps a [`Uid`] to an [`ItemId`] in the current save.
const GET_ITEM_ID: usize = 0x1_40C3_D680;
/// Address of the function that puts an item into one of the equipment slots.
const SET_SLOT: usize = 0x1_40D5_92F0;
/// Address of the static pointer to the game data singleton.
const GAME_DATA: usize = 0x1_43D5_AAC0;

/// Checks that the running executable could be the supported game version: it loaded at
/// [`IMAGE_BASE`], and every address the mod uses lies inside its image.
///
/// Returns the executable's build timestamp from its PE header, which tells game versions apart.
pub fn check_executable() -> anyhow::Result<u32> {
    let base = unsafe { GetModuleHandleW(PCWSTR::null()) }?.0 as usize;
    if base != IMAGE_BASE {
        bail!("the game's executable loaded at {base:#x} instead of {IMAGE_BASE:#x}");
    }
    // SAFETY: a loaded image starts with its DOS header, whose field at 0x3C is the offset of the
    // PE headers. Those hold the build timestamp at +0x8 and the image size at +0x50.
    let (timestamp, image_size) = unsafe {
        let image = base as *const u8;
        let headers = image.add(image.add(0x3C).cast::<u32>().read_unaligned() as usize);
        if headers.cast::<u32>().read_unaligned() != u32::from_le_bytes(*b"PE\0\0") {
            bail!("the game's executable has no PE headers");
        }
        (headers.add(0x8).cast::<u32>().read_unaligned(), headers.add(0x50).cast::<u32>().read_unaligned())
    };
    let image = base..base + image_size as usize;
    for address in [PROCESS_INPUT, GET_ITEM_ID, SET_SLOT, GAME_DATA] {
        if !image.contains(&address) {
            bail!("{address:#x} is outside the game's executable, which ends at {:#x}", image.end);
        }
    }
    Ok(timestamp)
}

/// Equipment slot index that holds the combat art.
const COMBAT_ART_SLOT: u32 = 1;

/// Actions the player requests in a frame, as bit flags. Bits without a constant here are kept
/// as they are.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actions(u64);

impl Actions {
    pub const ATTACK: Self = Self(0x1);
    pub const BLOCK: Self = Self(0x4);

    /// Returns whether every flag of `other` is set.
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}

impl BitOr for Actions {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Signature of the hooked input processing function at [`PROCESS_INPUT`].
pub type ProcessInputFn = unsafe extern "system" fn(*mut InputHandler, usize) -> usize;
type GetItemIdFn = unsafe extern "system" fn(inventory: *const c_void, uid: *const u32) -> u32;
type SetSlotFn = unsafe extern "system" fn(slot: u32, data: *const EquipData, ignore_lock: bool) -> u64;

/// The object passed to [`PROCESS_INPUT`] every frame.
#[repr(C)]
pub struct InputHandler {
    _pad: [u8; 0x10],
    /// Actions the player requests this frame.
    pub action: Actions,
}

#[repr(C)]
struct GameData {
    _pad: [u8; 0x8],
    player_data: *mut PlayerData,
}

#[repr(C)]
struct PlayerData {
    _pad0: [u8; 0x29C],
    /// Item ids in the equipment slots: prosthetic 1, combat art, prosthetic 2, unused, prosthetic 3.
    equipped_items: [u32; 5],
    _pad1: [u8; 0x300],
    inventory_data: *mut u8,
}

const _: () = {
    assert!(offset_of!(InputHandler, action) == 0x10);
    assert!(offset_of!(GameData, player_data) == 0x8);
    assert!(offset_of!(PlayerData, equipped_items) == 0x29C);
    assert!(offset_of!(PlayerData, inventory_data) == 0x5B0);
};

/// Argument of the slot setter: an item id at byte offset 56 with zeroed padding before it.
#[repr(C)]
struct EquipData {
    _pad: [u32; 14],
    item_id: u32,
}

/// Save-independent id of a combat art (or any other item), as listed in the game's params.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uid(pub u32);

/// Id of an item in the current save's inventory.
///
/// The same [`Uid`] maps to different item ids in different saves, so these must never be cached
/// across save loads. Putting an item id that the save doesn't own into a slot corrupts the
/// equipment (the player can lose the katana), which is why the only way to obtain one is
/// [`Player::item_id`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemId(NonZeroU32);

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Handle to the loaded player's data.
///
/// The game mutates this data on its main thread with no locking, so the handle can only be
/// obtained through [`Player::current`] on that thread and is neither `Send` nor `Sync`.
pub struct Player {
    data: NonNull<PlayerData>,
    _not_send: PhantomData<*const ()>,
}

impl Player {
    /// Returns the loaded player, or `None` while no save is loaded.
    ///
    /// # Safety
    ///
    /// Must be called on the game's main thread, such as from inside the [`PROCESS_INPUT`] hook,
    /// and the handle must not outlive the current frame.
    pub unsafe fn current() -> Option<Self> {
        let game_data = unsafe { (*(GAME_DATA as *const *const GameData)).as_ref()? };
        let data = NonNull::new(game_data.player_data)?;
        if unsafe { data.as_ref() }.inventory_data.is_null() {
            return None;
        }
        Some(Self { data, _not_send: PhantomData })
    }

    fn data(&self) -> &PlayerData {
        // SAFETY: `current` checked the pointer, and the game doesn't free it mid-frame.
        unsafe { self.data.as_ref() }
    }

    /// Returns the inventory item id of `uid`, or `None` if the save doesn't own that item.
    pub fn item_id(&self, uid: Uid) -> Option<ItemId> {
        let get_item_id: GetItemIdFn = unsafe { std::mem::transmute(GET_ITEM_ID) };
        // SAFETY: `inventory_data` is non-null, and the inventory lives at offset 0x10 of it.
        let inventory = unsafe { self.data().inventory_data.add(0x10) }.cast::<c_void>();
        let raw = unsafe { get_item_id(inventory, &uid.0) };
        // The function returns garbage above 0xFFFF for items the save doesn't have.
        NonZeroU32::new(raw).filter(|id| id.get() < 0xFFFF).map(ItemId)
    }

    /// Returns the raw item id currently in the combat art slot.
    pub fn equipped_art(&self) -> u32 {
        self.data().equipped_items[COMBAT_ART_SLOT as usize]
    }

    /// Returns whether `item` is the one in the combat art slot.
    pub fn has_equipped(&self, item: ItemId) -> bool {
        self.equipped_art() == item.0.get()
    }

    /// Puts `item` into the combat art slot immediately, even in the middle of an animation.
    pub fn equip_art(&self, item: ItemId) {
        let set_slot: SetSlotFn = unsafe { std::mem::transmute(SET_SLOT) };
        let data = EquipData { _pad: [0; 14], item_id: item.0.get() };
        // The game refuses every change outside the equipment menu unless its lock is ignored.
        // SAFETY: called on the main thread with a valid item id owned by the save.
        unsafe { set_slot(COMBAT_ART_SLOT, &data, true) };
    }
}
