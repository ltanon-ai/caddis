//! presets.rs — BC4 strategy presets + hysteresis. Presets ONLY until
//! determinism holds: same (goal_id, strategy) => same blast_set over
//! TAGGED rows (measured by ladder.determinism over the profile's
//! stamped rows — the strategy ledger). Free-form strategies stay
//! forbidden until that check holds across recorded history. The warden
//! envelope is NEVER extended with any of this.

pub const WEAK_FIRST: &str = "weak-first";
pub const STRONG_FIRST: &str = "strong-first";

/// The whole preset vocabulary (BC4): nothing else is dispatchable.
pub const PRESETS: &[&str] = &[WEAK_FIRST, STRONG_FIRST];

/// Hysteresis N=4: four consecutive non-accepts under the current preset
/// are a trend, not noise — fewer never switch.
pub const HYSTERESIS_N: u32 = 4;

/// One-way preset gate: starts weak-first; switches to strong-first after
/// four consecutive failed gates. Returning to weak-first is an operator
/// decision recorded as evidence, never an automatic flap.
#[derive(Debug, Clone, PartialEq)]
pub struct PresetGate {
    current: &'static str,
    fails: u32,
}

impl PresetGate {
    pub fn new() -> Self {
        Self {
            current: WEAK_FIRST,
            fails: 0,
        }
    }

    pub fn current(&self) -> &'static str {
        self.current
    }

    /// Feed each dispatch outcome; returns the preset in force after it.
    pub fn tick(&mut self, accepted: bool) -> &'static str {
        if accepted {
            self.fails = 0;
            return self.current;
        }
        self.fails += 1;
        if self.fails >= HYSTERESIS_N {
            self.current = STRONG_FIRST;
            self.fails = 0;
        }
        self.current
    }
}

impl Default for PresetGate {
    fn default() -> Self {
        Self::new()
    }
}
