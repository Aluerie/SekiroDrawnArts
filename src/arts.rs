//! The game's combat arts and how long each one keeps the player busy.

use std::time::Duration;

use crate::game::Uid;

/// The "no combat art" item that occupies the slot when nothing is equipped.
pub const EMPTY: Uid = Uid(5000);

/// A combat art the player can equip.
#[derive(Debug)]
pub struct CombatArt {
    pub uid: Uid,
    pub name: &'static str,
}

/// Every combat art in the game, in the order the in-game skill menu lists them.
pub const ALL: &[CombatArt] = &[
    art(5100, "Whirlwind Slash"),
    art(5200, "Nightjar Slash"),
    art(7000, "Nightjar Slash Reversal"),
    art(5300, "Ichimonji"),
    art(7100, "Ichimonji: Double"),
    art(5500, "Ashina Cross"),
    art(5600, "Floating Passage"),
    art(7200, "Spiral Cloud Passage"),
    art(5900, "Praying Strikes"),
    art(7500, "Praying Strikes - Exorcism"),
    art(5800, "Senpou Leaping Kicks"),
    art(7400, "High Monk"),
    art(6000, "Shadowrush"),
    art(7600, "Shadowfall"),
    art(7700, "Sakura Dance"),
    art(5700, "Mortal Draw"),
    art(7300, "Empowered Mortal Draw"),
    art(5400, "Dragon Flash"),
    art(6100, "One Mind"),
];

const fn art(uid: u32, name: &'static str) -> CombatArt {
    CombatArt { uid: Uid(uid), name }
}

impl CombatArt {
    /// Looks up a combat art by its uid.
    pub fn find(uid: Uid) -> Option<&'static Self> {
        ALL.iter().find(|art| art.uid == uid)
    }

    /// Returns whether the art keeps going for as long as Attack is held after it starts.
    pub fn is_sheathed(&self) -> bool {
        matches!(self.uid, Uid(5500 | 6100))
    }

    /// Time after starting the art during which the slot must not change.
    ///
    /// Swapping the art out earlier makes the game play animations of unrelated arts.
    pub fn recovery(&self) -> Duration {
        Duration::from_millis(match self.uid.0 {
            5500 => 1250,        // Ashina Cross
            5700 | 7300 => 2000, // Mortal Draw draws and sheathes a second sword
            6100 => 4000,        // One Mind
            7700 => 1000,        // Sakura Dance
            _ => 700,
        })
    }
}
