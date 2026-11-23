//! The per-frame logic: keeps the selected combat art equipped only while the hold key is down.

use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use crate::{
    arts::{self, CombatArt},
    camera,
    config::Config,
    game::{Actions, ItemId, Player, Uid},
    input::{self, Button, MouseTracker, Transition},
    wheel::{SharedWheel, Wheel},
};

/// How long Attack is withheld from the game after equipping an art.
///
/// If Block + Attack reaches the game right after the slot changes, it performs Whirlwind Slash
/// instead of the art that was just equipped.
const ATTACK_SUPPRESSION: Duration = Duration::from_millis(50);

/// How long the art stays equipped after the hold key is released. Pressing the key again within
/// this time leaves the slot untouched, so tapping the key repeatedly doesn't swap the art in and
/// out on every press.
const RELEASE_GRACE: Duration = Duration::from_millis(250);

/// Frames during which Block is pressed for the player after Attack, so the game sees Block +
/// Attack and performs the art.
const BLOCK_INJECTION_FRAMES: u8 = 10;

/// The hold key as far as the slot is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    Up,
    Down,
    /// Released at this moment, less than [`RELEASE_GRACE`] ago.
    Releasing(Instant),
}

impl Hold {
    /// Whether the selected art should be in the slot.
    fn wants_art(self) -> bool {
        self != Hold::Up
    }
}

/// An art the player started, which must stay in the slot until its animation is over.
#[derive(Debug, Clone, Copy)]
struct Playing {
    art: &'static CombatArt,
    until: Instant,
    /// Whether Attack has been held without a break since a sheathed art started. The art keeps
    /// going until Attack is released, even if the hold key is released first.
    stance: bool,
}

/// Block pressed on the player's behalf so that Attack alone performs the art.
#[derive(Debug, Clone, Copy)]
enum BlockInjection {
    /// Keep pressing Block for this many more frames.
    Frames(u8),
    /// Keep pressing Block until Attack is released, for arts that last as long as Attack is held.
    WhileAttackHeld,
}

/// State of the mod carried between frames.
pub struct Controller {
    config: Config,
    hold_key: Button,
    wheel_key: Button,
    hold: Hold,
    /// The wheel shared with the overlay that draws it.
    wheel: SharedWheel,
    /// Steers the wheel while it's open.
    mouse: MouseTracker,
    /// Whether any mouse movement has steered the wheel yet, logged once to help diagnose input.
    mouse_seen: bool,
    /// The art equipped while the hold key is down.
    selected: Uid,
    /// The art currently playing, during which the slot must not change.
    playing: Option<Playing>,
    /// The slot's raw contents and the art they were found to be, so the game isn't asked for
    /// every art's item id on every frame. Item ids differ between saves, so this is cleared on
    /// every save load.
    equipped_cache: Option<(u32, Option<&'static CombatArt>)>,
    /// The item this mod last asked the game to put into the slot.
    requested: Option<ItemId>,
    /// The raw slot contents last frame, to notice changes the game makes on its own, such as
    /// through the equipment menu or by loading a save.
    slot_seen: Option<u32>,
    /// An item put into the slot last frame, checked this frame to see whether the game took it.
    unconfirmed: Option<ItemId>,
    attack_was_down: bool,
    suppress_attack_until: Option<Instant>,
    block_injection: Option<BlockInjection>,
    /// Arts already reported as not owned, so the log isn't flooded every frame.
    reported_missing: HashSet<Uid>,
}

impl Controller {
    pub fn new(config: Config, wheel: SharedWheel) -> Self {
        Self {
            hold_key: Button::new(config.hold_key),
            wheel_key: Button::new(config.wheel_key),
            hold: Hold::Up,
            wheel,
            mouse: MouseTracker::default(),
            mouse_seen: false,
            selected: config.default_art,
            config,
            playing: None,
            equipped_cache: None,
            requested: None,
            slot_seen: None,
            unconfirmed: None,
            attack_was_down: false,
            suppress_attack_until: None,
            block_injection: None,
            reported_missing: HashSet::new(),
        }
    }

    /// Forgets the previous save's state and logs what the mod finds in the new one.
    pub fn on_save_loaded(&mut self, player: &Player) {
        self.reported_missing.clear();
        self.playing = None;
        self.equipped_cache = None;
        self.requested = None;
        self.slot_seen = None;
        self.unconfirmed = None;
        self.attack_was_down = false;
        self.suppress_attack_until = None;
        self.block_injection = None;
        let describe = |item: Option<ItemId>| item.map_or("not owned".to_owned(), |item| format!("item {item}"));
        log::info!(
            "save loaded: slot holds item {}, empty art is {}, {} is {}",
            player.equipped_art(),
            describe(player.item_id(arts::EMPTY)),
            CombatArt::find(self.selected).map_or("?", |art| art.name),
            describe(player.item_id(self.selected)),
        );
        let owned: Vec<_> =
            arts::ALL.iter().filter(|art| player.item_id(art.uid).is_some()).map(|art| art.name).collect();
        log::info!("owned combat arts: {}", if owned.is_empty() { "none".to_owned() } else { owned.join(", ") });
    }

    /// Runs once per frame on the game thread, before the game reads the player's `actions`.
    pub fn tick(&mut self, player: &Player, actions: &mut Actions) {
        let now = Instant::now();
        let focused = input::game_focused();
        self.update_hold(focused, now);
        let wheel_key = self.wheel_key.poll(focused);

        if let Some(item) = self.unconfirmed.take().filter(|&item| !player.has_equipped(item)) {
            log::warn!("the game ignored equipping item {item}; the slot holds item {}", player.equipped_art());
        }

        self.adopt_external_change(player);
        if self.update_wheel(player, wheel_key) {
            // Clicking while choosing must not swing the sword.
            actions.remove(Actions::ATTACK);
        }

        if let Some(done) = self.playing.take_if(|playing| now >= playing.until) {
            log::debug!("{} finished, the slot can change again", done.art.name);
        }
        if self.playing.is_none() {
            self.sync_slot(player, now);
        }

        if self.suppress_attack_until.is_some_and(|until| now < until) {
            actions.remove(Actions::ATTACK);
        }

        // Edges of Attack as the game sees it, after the suppression above.
        let attacking = actions.contains(Actions::ATTACK);
        let attack_pressed = attacking && !self.attack_was_down;
        self.attack_was_down = attacking;

        let equipped = self.equipped_art(player);
        self.inject_block(equipped, actions, attack_pressed);
        if let Some(playing) = &mut self.playing {
            playing.stance &= actions.contains(Actions::ATTACK);
        }
        self.hold_back_lingering_art(equipped, actions, attack_pressed);
        // Only a press that still reaches the game can start an art.
        let attack_pressed = attack_pressed && actions.contains(Actions::ATTACK);
        self.track_art_usage(equipped, *actions, attack_pressed, now);
    }

    fn update_hold(&mut self, focused: bool, now: Instant) {
        match self.hold_key.poll(focused) {
            Transition::Pressed => {
                log::debug!("hold key pressed");
                self.hold = Hold::Down;
            }
            Transition::Released => {
                log::debug!("hold key released");
                self.hold = Hold::Releasing(now);
            }
            Transition::Held | Transition::Idle => {}
        }
        if let Hold::Releasing(at) = self.hold
            && now - at >= RELEASE_GRACE
        {
            self.hold = Hold::Up;
        }
    }

    /// Presses Block along with Attack while the hold key is down, so hold key + Attack performs
    /// the equipped art without the player pressing Block.
    ///
    /// Not done while an art is still playing: the game queues the extra art inputs, which
    /// stretches the art past its recovery window and lets the slot change mid-animation.
    fn inject_block(&mut self, equipped: Option<&'static CombatArt>, actions: &mut Actions, attack_pressed: bool) {
        if attack_pressed
            && self.hold == Hold::Down
            && self.playing.is_none()
            && !actions.contains(Actions::BLOCK)
            && let Some(art) = equipped
        {
            log::debug!("{} triggered by hold key + Attack, pressing Block too", art.name);
            self.block_injection = Some(if art.is_sheathed() {
                BlockInjection::WhileAttackHeld
            } else {
                BlockInjection::Frames(BLOCK_INJECTION_FRAMES)
            });
        }
        self.block_injection = match self.block_injection {
            Some(BlockInjection::Frames(frames)) => {
                actions.insert(Actions::BLOCK);
                frames.checked_sub(1).filter(|&left| left > 0).map(BlockInjection::Frames)
            }
            Some(BlockInjection::WhileAttackHeld) if actions.contains(Actions::ATTACK) => {
                actions.insert(Actions::BLOCK);
                Some(BlockInjection::WhileAttackHeld)
            }
            Some(BlockInjection::WhileAttackHeld) | None => None,
        };
    }

    /// Withholds Attack from Block + Attack while an art is still in the slot but the hold key is
    /// up, which happens until the previous art finishes playing. Otherwise the art would come out
    /// without the hold key.
    ///
    /// A sheathed art whose Attack is still held is left alone: withholding Attack would unleash
    /// it as soon as the hold key is released.
    fn hold_back_lingering_art(
        &self,
        equipped: Option<&'static CombatArt>,
        actions: &mut Actions,
        attack_pressed: bool,
    ) {
        let in_stance = self.playing.is_some_and(|playing| playing.stance);
        if in_stance
            || self.hold == Hold::Down
            || equipped.is_none()
            || !actions.contains(Actions::BLOCK | Actions::ATTACK)
        {
            return;
        }
        if attack_pressed {
            log::debug!("held back Block + Attack: the art is still equipped but the hold key is up");
        }
        actions.remove(Actions::ATTACK);
    }

    /// Starts the recovery window when the player performs the equipped art.
    fn track_art_usage(
        &mut self,
        equipped: Option<&'static CombatArt>,
        actions: Actions,
        attack_pressed: bool,
        now: Instant,
    ) {
        let Some(art) = equipped else { return };
        if attack_pressed && actions.contains(Actions::BLOCK) {
            log::debug!("{} performed, keeping the slot for at least {:?}", art.name, art.recovery());
            self.playing = Some(Playing { art, until: now + art.recovery(), stance: art.is_sheathed() });
        } else if let Some(playing) = &mut self.playing
            && playing.stance
        {
            // Sheathed arts last for as long as Attack is held, so their window restarts every frame.
            playing.until = now + playing.art.recovery();
        }
    }

    /// Makes an art equipped from the in-game menu, or found in a freshly loaded save, the
    /// selected one, so it's what the hold key equips from then on.
    fn adopt_external_change(&mut self, player: &Player) {
        let in_slot = player.equipped_art();
        let changed = self.slot_seen.replace(in_slot) != Some(in_slot);
        if !changed || self.requested.is_some_and(|item| player.has_equipped(item)) {
            return;
        }
        if player.item_id(arts::EMPTY).is_some_and(|item| player.has_equipped(item)) {
            return;
        }
        match self.equipped_art(player) {
            Some(art) if art.uid != self.selected => {
                log::info!("{} was equipped outside the mod, selecting it", art.name);
                self.selected = art.uid;
            }
            Some(_) => {}
            None => log::debug!("unknown item {in_slot} in the combat art slot"),
        }
    }

    /// Opens, steers and closes the wheel, and returns whether it's open.
    fn update_wheel(&mut self, player: &Player, transition: Transition) -> bool {
        if transition == Transition::Pressed {
            let entries = self.wheel_entries(player);
            log::debug!("wheel opened with {} arts", entries.len());
            *self.wheel.lock() = Some(Wheel::new(entries, self.selected));
            camera::freeze(true);
            if !camera::is_active() {
                self.mouse.start();
            }
        }

        let mut shared = self.wheel.lock();
        match transition {
            Transition::Held => {
                if let Some(wheel) = shared.as_mut() {
                    wheel.touch();
                    // The game's raw mouse input is precise and unaccelerated; the cursor is only
                    // a fallback for when the game doesn't read the mouse through DirectInput.
                    let raw = camera::is_active();
                    let motion = if raw {
                        let sensitivity = self.config.wheel_sensitivity;
                        camera::take_motion().map(|(dx, dy)| (dx * sensitivity, dy * sensitivity))
                    } else {
                        self.mouse.motion()
                    };
                    if let Some((dx, dy)) = motion {
                        if !self.mouse_seen {
                            self.mouse_seen = true;
                            let source = if raw { "DirectInput" } else { "the cursor" };
                            log::info!("mouse movement reaches the wheel through {source}");
                        }
                        wheel.add_mouse_motion(dx, dy);
                    }
                }
            }
            Transition::Released => {
                self.mouse.stop();
                camera::freeze(false);
                match shared.take().and_then(|wheel| wheel.hovered_art()) {
                    Some(art) => {
                        log::info!("selected {} from the wheel", art.name);
                        self.selected = art.uid;
                    }
                    None => log::debug!("wheel closed without a choice"),
                }
            }
            Transition::Pressed | Transition::Idle => {}
        }
        shared.is_some()
    }

    /// Arts to show on the wheel: the configured ones, or every art, limited to what the save owns.
    fn wheel_entries(&self, player: &Player) -> Vec<&'static CombatArt> {
        let owned = |art: &&CombatArt| player.item_id(art.uid).is_some();
        if self.config.wheel_arts.is_empty() {
            arts::ALL.iter().filter(owned).collect()
        } else {
            self.config.wheel_arts.iter().filter_map(|&uid| CombatArt::find(uid)).filter(owned).collect()
        }
    }

    /// Puts the selected art into the slot while the art is wanted, and empties it otherwise.
    /// Not called while an art is still playing, so it's never pulled out mid-animation.
    fn sync_slot(&mut self, player: &Player, now: Instant) {
        let Some(empty) = self.owned_item(player, arts::EMPTY) else { return };
        let wanted =
            if self.hold.wants_art() { self.owned_item(player, self.selected).unwrap_or(empty) } else { empty };
        self.requested = Some(wanted);
        if player.has_equipped(wanted) {
            return;
        }

        if wanted == empty {
            log::debug!("emptying the slot (held item {})", player.equipped_art());
        } else {
            log::debug!("equipping item {wanted} (slot held item {})", player.equipped_art());
            self.suppress_attack_until = Some(now + ATTACK_SUPPRESSION);
        }
        player.equip_art(wanted);
        self.unconfirmed = Some(wanted);
    }

    /// Returns the save's item id for `uid`, logging once per art if the save doesn't own it.
    fn owned_item(&mut self, player: &Player, uid: Uid) -> Option<ItemId> {
        let item = player.item_id(uid);
        if item.is_none() && self.reported_missing.insert(uid) {
            let name = CombatArt::find(uid).map_or("the empty combat art", |art| art.name);
            log::warn!("the loaded save doesn't have {name} ({})", uid.0);
        }
        item
    }

    /// The art in the slot, or `None` if the slot is empty or holds something unknown.
    fn equipped_art(&mut self, player: &Player) -> Option<&'static CombatArt> {
        let in_slot = player.equipped_art();
        if let Some((cached_slot, art)) = self.equipped_cache
            && cached_slot == in_slot
        {
            return art;
        }
        let art = arts::ALL.iter().find(|art| player.item_id(art.uid).is_some_and(|item| player.has_equipped(item)));
        self.equipped_cache = Some((in_slot, art));
        art
    }
}
