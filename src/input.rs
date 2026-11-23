//! Keyboard polling with press/release detection.

use windows::Win32::{
    Foundation::{POINT, RECT},
    Graphics::Gdi::ClientToScreen,
    System::Threading::GetCurrentProcessId,
    UI::{
        Input::KeyboardAndMouse::{GetAsyncKeyState, VIRTUAL_KEY},
        WindowsAndMessaging::{
            GetClientRect, GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId, SetCursorPos,
        },
    },
};

/// How a key's state changed since the previous poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    Pressed,
    Held,
    Released,
    Idle,
}

/// A key whose state is polled once per frame.
#[derive(Debug)]
pub struct Button {
    key: VIRTUAL_KEY,
    was_down: bool,
}

impl Button {
    pub const fn new(key: VIRTUAL_KEY) -> Self {
        Self { key, was_down: false }
    }

    /// Reads the key's state and returns how it changed since the previous call.
    ///
    /// Keys count as released while the game window is not focused, so typing elsewhere
    /// doesn't trigger anything.
    pub fn poll(&mut self, focused: bool) -> Transition {
        let down = focused && is_down(self.key);
        let transition = match (self.was_down, down) {
            (false, true) => Transition::Pressed,
            (true, true) => Transition::Held,
            (true, false) => Transition::Released,
            (false, false) => Transition::Idle,
        };
        self.was_down = down;
        transition
    }
}

/// Returns whether `key` is currently held down.
pub fn is_down(key: VIRTUAL_KEY) -> bool {
    // The most significant bit is set while the key is down.
    let state = unsafe { GetAsyncKeyState(key.0.into()) };
    state < 0
}

/// Measures mouse movement from the cursor position, putting the cursor back at the centre of the
/// game window after every reading so it never stops at the screen's edge.
#[derive(Debug, Default)]
pub struct MouseTracker {
    anchor: Option<POINT>,
}

impl MouseTracker {
    /// Starts measuring from the centre of the foreground window.
    pub fn start(&mut self) {
        let window = unsafe { GetForegroundWindow() };
        let mut rect = RECT::default();
        if unsafe { GetClientRect(window, &mut rect) }.is_err() {
            return;
        }
        let mut center = POINT { x: (rect.left + rect.right) / 2, y: (rect.top + rect.bottom) / 2 };
        if unsafe { ClientToScreen(window, &mut center) }.as_bool() {
            let _ = unsafe { SetCursorPos(center.x, center.y) };
            self.anchor = Some(center);
        }
    }

    pub fn stop(&mut self) {
        self.anchor = None;
    }

    /// Returns how far the cursor moved since the last call, in pixels, if it moved at all.
    pub fn motion(&mut self) -> Option<(f32, f32)> {
        let anchor = self.anchor?;
        let mut pos = POINT::default();
        unsafe { GetCursorPos(&mut pos) }.ok()?;
        let (dx, dy) = (pos.x - anchor.x, pos.y - anchor.y);
        if dx == 0 && dy == 0 {
            return None;
        }
        let _ = unsafe { SetCursorPos(anchor.x, anchor.y) };
        Some((dx as f32, dy as f32))
    }
}

/// Returns whether the foreground window belongs to this process.
pub fn game_focused() -> bool {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
    pid == unsafe { GetCurrentProcessId() }
}
