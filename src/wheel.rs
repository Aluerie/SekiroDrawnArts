//! The combat art wheel: its state shared between threads, and its imgui overlay.

use std::{
    f32::consts::{PI, TAU},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, Once, PoisonError},
    time::{Duration, Instant},
};

use hudhook::{
    ImguiRenderLoop, RenderContext,
    imgui::{self, FontConfig, FontSource},
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CYSCREEN};

use crate::{arts::CombatArt, game::Uid, icons::Icons};

/// Mouse travel, in pixels, needed before the wheel starts following the mouse.
const AIM_DEADZONE: f32 = 40.0;
/// Mouse travel, in pixels, beyond which further movement in the same direction is ignored,
/// so turning the aim around never takes long.
const AIM_LIMIT: f32 = 150.0;
/// How far past the edge of the pointed-at slot the aim must go before the neighbouring slot is
/// pointed at instead, as a fraction of a slot's width. Keeps the choice steady near a boundary.
const SLOT_HYSTERESIS: f32 = 0.2;
/// How far from the centre the pointer can travel, as a fraction of the wheel's radius. It stays
/// inside the ring of slots.
const POINTER_REACH: f32 = 0.55;
/// How long the wheel stays drawn without the game processing input, such as in a menu.
const STALE_AFTER: Duration = Duration::from_millis(200);

/// Fonts to try, in order, from the Windows fonts folder: Palatino Linotype Bold, Georgia Bold
/// and Segoe UI Semibold.
const FONT_FILES: [&str; 3] = ["palab.ttf", "georgiab.ttf", "seguisb.ttf"];

/// The wheel while it's open and `None` while it's closed, shared between the game thread, which
/// opens, steers and closes it, and the render thread, which draws it.
#[derive(Debug, Clone, Default)]
pub struct SharedWheel(Arc<Mutex<Option<Wheel>>>);

impl SharedWheel {
    pub fn lock(&self) -> MutexGuard<'_, Option<Wheel>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// An open wheel: what it shows and which entry is pointed at.
#[derive(Debug)]
pub struct Wheel {
    entries: Vec<&'static CombatArt>,
    /// Index into `entries` of the currently selected art, highlighted differently from the aim.
    current: Option<usize>,
    /// Accumulated mouse movement since the wheel opened.
    aim: [f32; 2],
    /// Index into `entries` of the entry being pointed at.
    hovered: Option<usize>,
    last_seen: Instant,
}

impl Wheel {
    /// Shows `entries` around the wheel, with `current` marked as the active one.
    pub fn new(entries: Vec<&'static CombatArt>, current: Uid) -> Self {
        Self {
            current: entries.iter().position(|art| art.uid == current),
            entries,
            aim: [0.0; 2],
            hovered: None,
            last_seen: Instant::now(),
        }
    }

    /// The art being pointed at, if any.
    pub fn hovered_art(&self) -> Option<&'static CombatArt> {
        self.hovered().map(|i| self.entries[i])
    }

    /// Records that the game thread is still driving the wheel this frame.
    pub fn touch(&mut self) {
        self.last_seen = Instant::now();
    }

    /// Moves the aim by a mouse movement of `(dx, dy)` pixels, where `dy` grows downwards.
    pub fn add_mouse_motion(&mut self, dx: f32, dy: f32) {
        let [x, y] = [self.aim[0] + dx, self.aim[1] + dy];
        let scale = (AIM_LIMIT / x.hypot(y)).min(1.0);
        self.aim = [x * scale, y * scale];
        let count = self.entries.len();
        self.hovered = match self.hovered {
            _ if x.hypot(y) <= AIM_DEADZONE => None,
            Some(index) if slot_offset(x, y, index, count) < 0.5 + SLOT_HYSTERESIS => Some(index),
            _ => slot_at(x, y, count),
        };
    }

    /// Index of the entry being pointed at.
    fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// Whether the game thread stopped driving the wheel, such as while a menu is open.
    fn is_stale(&self) -> bool {
        self.last_seen.elapsed() >= STALE_AFTER
    }
}

/// Returns the slot that screen direction `(x, y)` points at on a wheel of `count` slots laid out
/// clockwise starting from the top, or `None` for a zero direction or an empty wheel.
fn slot_at(x: f32, y: f32, count: usize) -> Option<usize> {
    if count == 0 || (x == 0.0 && y == 0.0) {
        return None;
    }
    let angle = x.atan2(-y).rem_euclid(TAU);
    let step = TAU / count as f32;
    Some((angle / step).round() as usize % count)
}

/// How far screen direction `(x, y)` is from the centre of slot `index` on a wheel of `count`
/// slots, measured in slot widths.
fn slot_offset(x: f32, y: f32, index: usize, count: usize) -> f32 {
    let step = TAU / count as f32;
    let angle = x.atan2(-y) - index as f32 * step;
    // Wrap into -PI..PI so the distance is measured the short way around.
    ((angle + PI).rem_euclid(TAU) - PI).abs() / step
}

/// Screen direction of slot `index` on a wheel of `count` slots, as a unit vector.
fn slot_direction(index: usize, count: usize) -> [f32; 2] {
    let angle = index as f32 * TAU / count as f32;
    [angle.sin(), -angle.cos()]
}

/// Fonts for the art names under each slot and for the pointed-at art in the centre, as indices
/// into the font atlas, since imgui's font handles can't be shared between threads.
#[derive(Clone, Copy)]
struct Fonts {
    label: usize,
    title: usize,
}

impl Fonts {
    /// Adds the first available font from [`FONT_FILES`] in two sizes scaled to the screen height,
    /// or returns `None` to keep imgui's built-in font.
    fn load(ctx: &mut imgui::Context) -> Option<Self> {
        let fonts_dir =
            std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from).join("Fonts");
        let (path, data) = FONT_FILES.iter().find_map(|file| {
            let path = fonts_dir.join(file);
            fs::read(&path).ok().map(|data| (path, data))
        })?;
        let screen_height = unsafe { GetSystemMetrics(SM_CYSCREEN) }.max(720) as f32;
        let mut add = |size_pixels: f32| {
            let config = FontConfig { oversample_h: 2, oversample_v: 2, ..FontConfig::default() };
            let atlas = ctx.fonts();
            let id = atlas.add_font(&[FontSource::TtfData { data: &data, size_pixels, config: Some(config) }]);
            atlas.fonts().iter().position(|&font| font == id).unwrap_or_default()
        };
        let fonts = Self { label: add(screen_height / 40.0), title: add(screen_height / 26.0) };
        log::info!("wheel font: {}", display_name(&path));
        Some(fonts)
    }
}

fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// The imgui overlay that draws the wheel.
pub struct Overlay {
    wheel: SharedWheel,
    icon_dir: PathBuf,
    icons: Icons,
    fonts: Option<Fonts>,
}

impl Overlay {
    /// Creates the overlay that draws `wheel`, with the combat art images found in `icon_dir`.
    pub fn new(wheel: SharedWheel, icon_dir: PathBuf) -> Self {
        Self { wheel, icon_dir, icons: Icons::default(), fonts: None }
    }
}

impl ImguiRenderLoop for Overlay {
    fn initialize<'a>(&'a mut self, ctx: &mut imgui::Context, render_context: &'a mut dyn RenderContext) {
        self.fonts = Fonts::load(ctx);
        if self.fonts.is_none() {
            log::warn!("none of {FONT_FILES:?} found, using imgui's built-in font");
        }
        self.icons = Icons::load(&self.icon_dir, render_context);
    }

    fn render(&mut self, ui: &mut imgui::Ui) {
        static FIRST_FRAME: Once = Once::new();
        FIRST_FRAME.call_once(|| log::info!("the wheel overlay is drawing on the game's frames"));
        if let Some(wheel) = self.wheel.lock().as_ref().filter(|wheel| !wheel.is_stale()) {
            draw(ui, wheel, &self.icons, self.fonts);
        }
    }
}

mod color {
    pub const BACKDROP: [f32; 4] = [0.05, 0.04, 0.04, 0.6];
    pub const SLOT: [f32; 4] = [0.12, 0.10, 0.09, 0.85];
    pub const SLOT_HOVERED: [f32; 4] = [0.62, 0.16, 0.12, 0.95];
    pub const RING_CURRENT: [f32; 4] = [0.85, 0.72, 0.45, 1.0];
    pub const TEXT: [f32; 4] = [0.93, 0.90, 0.84, 1.0];
    pub const TEXT_DIM: [f32; 4] = [0.65, 0.62, 0.58, 1.0];
    pub const SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.85];
    pub const POINTER: [f32; 4] = [0.95, 0.80, 0.50, 1.0];
    pub const POINTER_IDLE: [f32; 4] = [0.65, 0.62, 0.58, 0.8];
    pub const POINTER_TRACK: [f32; 4] = [0.93, 0.90, 0.84, 0.15];
}

fn draw(ui: &imgui::Ui, wheel: &Wheel, icons: &Icons, fonts: Option<Fonts>) {
    let [width, height] = ui.io().display_size;
    let center = [width / 2.0, height / 2.0];
    let radius = height * 0.28;
    let count = wheel.entries.len();
    let slot_radius = (radius * 0.9 * (TAU / count.max(6) as f32) / 2.0).min(radius * 0.32);
    let hovered = wheel.hovered();
    let draw_list = ui.get_foreground_draw_list();

    draw_list.add_circle(center, radius + slot_radius * 1.6, color::BACKDROP).filled(true).num_segments(128).build();

    let font = |index: usize| ui.fonts().fonts().get(index).map(|&id| ui.push_font(id));
    let label_font = fonts.and_then(|f| font(f.label));
    if count == 0 {
        centered_text(ui, &draw_list, center, "No combat arts unlocked", color::TEXT_DIM);
        return;
    }

    for (index, art) in wheel.entries.iter().enumerate() {
        let [dx, dy] = slot_direction(index, count);
        let pos = [center[0] + dx * radius, center[1] + dy * radius];
        let is_hovered = hovered == Some(index);
        let fill = if is_hovered { color::SLOT_HOVERED } else { color::SLOT };
        draw_list.add_circle(pos, slot_radius, fill).filled(true).num_segments(48).build();
        if wheel.current == Some(index) {
            draw_list.add_circle(pos, slot_radius, color::RING_CURRENT).thickness(3.0).num_segments(48).build();
        }
        if let Some(texture) = icons.get(art.uid) {
            draw_icon(&draw_list, texture, pos, slot_radius * 0.85);
        }
        let below = [pos[0], pos[1] + slot_radius + ui.text_line_height() * 0.75];
        let tint = if is_hovered { color::TEXT } else { color::TEXT_DIM };
        centered_text(ui, &draw_list, below, short_name(art.name), tint);
    }
    drop(label_font);

    let _title_font = fonts.and_then(|f| font(f.title));
    let hovered = hovered.map(|index| wheel.entries[index]);
    let (title, tint) = match hovered {
        Some(art) => (art.name, color::TEXT),
        None => ("Move the mouse to choose", color::TEXT_DIM),
    };
    match hovered.and_then(|art| icons.get(art.uid)) {
        Some(texture) => {
            let icon_radius = radius * 0.32;
            draw_icon(&draw_list, texture, [center[0], center[1] - icon_radius * 0.3], icon_radius);
            let below = [center[0], center[1] + icon_radius * 0.7 + ui.text_line_height() * 0.6];
            centered_text(ui, &draw_list, below, title, tint);
        }
        None => centered_text(ui, &draw_list, center, title, tint),
    }

    draw_pointer(&draw_list, wheel, center, radius);
}

/// Draws the aim as a dot inside the wheel, on a faint ring marking how far it can travel. The dot
/// lights up once it's far enough from the centre to point at a slot.
fn draw_pointer(draw_list: &imgui::DrawListMut<'_>, wheel: &Wheel, center: [f32; 2], radius: f32) {
    let reach = radius * POINTER_REACH;
    let scale = reach / AIM_LIMIT;
    let [x, y] = wheel.aim;
    let pos = [center[0] + x * scale, center[1] + y * scale];
    let size = radius * 0.035;
    let fill = if wheel.hovered().is_some() { color::POINTER } else { color::POINTER_IDLE };
    draw_list.add_circle(center, reach, color::POINTER_TRACK).thickness(1.5).num_segments(96).build();
    draw_list.add_circle(center, AIM_DEADZONE * scale, color::POINTER_TRACK).thickness(1.0).num_segments(48).build();
    draw_list.add_circle(pos, size + 2.0, color::SHADOW).filled(true).num_segments(24).build();
    draw_list.add_circle(pos, size, fill).filled(true).num_segments(24).build();
}

/// Draws a texture in the square of half-width `radius` around `center`.
fn draw_icon(draw_list: &imgui::DrawListMut<'_>, texture: imgui::TextureId, center: [f32; 2], radius: f32) {
    let min = [center[0] - radius, center[1] - radius];
    let max = [center[0] + radius, center[1] + radius];
    draw_list.add_image(texture, min, max).build();
}

/// Shortens long names so neighbouring labels don't overlap.
fn short_name(name: &str) -> &str {
    match name {
        "Praying Strikes - Exorcism" => "Exorcism",
        "Nightjar Slash Reversal" => "Nightjar Reversal",
        "Empowered Mortal Draw" => "Empowered Draw",
        _ => name,
    }
}

/// Draws `text` centred on `at` in the current font, with a drop shadow for legibility.
fn centered_text(ui: &imgui::Ui, draw_list: &imgui::DrawListMut<'_>, at: [f32; 2], text: &str, tint: [f32; 4]) {
    let [w, h] = ui.calc_text_size(text);
    let top_left = [at[0] - w / 2.0, at[1] - h / 2.0];
    let offset = (h / 16.0).max(1.0);
    draw_list.add_text([top_left[0] + offset, top_left[1] + offset], color::SHADOW, text);
    draw_list.add_text(top_left, tint, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_start_at_top_and_go_clockwise() {
        assert_eq!(slot_at(0.0, -1.0, 4), Some(0));
        assert_eq!(slot_at(1.0, 0.0, 4), Some(1));
        assert_eq!(slot_at(0.0, 1.0, 4), Some(2));
        assert_eq!(slot_at(-1.0, 0.0, 4), Some(3));
        assert_eq!(slot_at(-0.1, -1.0, 4), Some(0));
    }

    #[test]
    fn slot_direction_round_trips() {
        for count in 1..=19 {
            for index in 0..count {
                let [x, y] = slot_direction(index, count);
                assert_eq!(slot_at(x, y, count), Some(index));
            }
        }
    }

    #[test]
    fn no_slot_without_direction_or_entries() {
        assert_eq!(slot_at(0.0, 0.0, 4), None);
        assert_eq!(slot_at(1.0, 0.0, 0), None);
    }

    #[test]
    fn pointed_slot_holds_until_clearly_past_its_edge() {
        let mut wheel = Wheel::new(crate::arts::ALL[..4].iter().collect(), Uid(0));
        wheel.add_mouse_motion(0.0, -100.0);
        assert_eq!(wheel.hovered(), Some(0));
        // Just past the 45° boundary with slot 1: still slot 0.
        wheel.add_mouse_motion(110.0, 0.0);
        assert_eq!(wheel.hovered(), Some(0));
        // Well into slot 1.
        wheel.add_mouse_motion(200.0, 100.0);
        assert_eq!(wheel.hovered(), Some(1));
    }

    #[test]
    fn mouse_points_once_past_deadzone() {
        let mut wheel = Wheel::new(crate::arts::ALL[..4].iter().collect(), Uid(0));
        wheel.add_mouse_motion(5.0, 0.0);
        assert_eq!(wheel.hovered(), None);
        wheel.add_mouse_motion(0.0, -400.0);
        assert_eq!(wheel.hovered(), Some(0));
        assert_eq!(wheel.hovered_art().map(|art| art.uid), Some(crate::arts::ALL[0].uid));
    }
}
