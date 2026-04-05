/// Parsed hotkey configuration.
pub struct HotkeyConfig {
    pub key: String,
}

impl HotkeyConfig {
    pub fn parse(s: &str) -> Self {
        Self { key: s.to_owned() }
    }
}

/// Runtime state for a push-to-talk or toggle hotkey.
pub struct HotkeyState {
    pub pressed: bool,
}

impl HotkeyState {
    pub fn new() -> Self {
        Self { pressed: false }
    }

    /// Mark the hotkey as pressed. Returns `true` (the new pressed state).
    pub fn press(&mut self) -> bool {
        self.pressed = true;
        true
    }

    /// Mark the hotkey as released. Returns `false` (the new pressed state).
    pub fn release(&mut self) -> bool {
        self.pressed = false;
        false
    }

    /// Toggle the pressed state. Returns the new state.
    pub fn toggle(&mut self) -> bool {
        self.pressed = !self.pressed;
        self.pressed
    }
}

impl Default for HotkeyState {
    fn default() -> Self {
        Self::new()
    }
}
