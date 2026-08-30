//! harness_driver.rs — CARD-0251. One interface, any agent CLI.
//!
//! The kernel owns the agent; the harness is a driver. Adding a new
//! harness = one struct + one registry line, not an if/else rewrite.

use std::process::{Command, Output};

/// A harness driver spawns a one-shot agent CLI with a prompt+model.
///
/// The warden judges the OUTPUT the same way regardless of driver —
/// the law is above the harness.
pub trait HarnessDriver {
    fn spawn(&self, prompt: &str, model: &str) -> Result<Output, String>;
    fn name(&self) -> &str;
}

/// `omp -p --model M`
pub struct OmpDriver;
impl HarnessDriver for OmpDriver {
    fn name(&self) -> &str {
        "omp"
    }
    fn spawn(&self, prompt: &str, model: &str) -> Result<Output, String> {
        Command::new("omp")
            .args(["-p", "--model", model, prompt])
            .output()
            .map_err(|e| format!("omp spawn: {e}"))
    }
}

/// `droid exec -m M`
pub struct DroidDriver;
impl HarnessDriver for DroidDriver {
    fn name(&self) -> &str {
        "droid"
    }
    fn spawn(&self, prompt: &str, model: &str) -> Result<Output, String> {
        Command::new("droid")
            .args(["exec", "-m", model, prompt])
            .output()
            .map_err(|e| format!("droid spawn: {e}"))
    }
}

/// `pi -p --model M`
pub struct PiDriver;
impl HarnessDriver for PiDriver {
    fn name(&self) -> &str {
        "pi"
    }
    fn spawn(&self, prompt: &str, model: &str) -> Result<Output, String> {
        Command::new("pi")
            .args(["-p", "--model", model, prompt])
            .output()
            .map_err(|e| format!("pi spawn: {e}"))
    }
}

/// `claude -p --model M`
pub struct ClaudeDriver;
impl HarnessDriver for ClaudeDriver {
    fn name(&self) -> &str {
        "claude"
    }
    fn spawn(&self, prompt: &str, model: &str) -> Result<Output, String> {
        Command::new("claude")
            .args(["-p", "--model", model, prompt])
            .output()
            .map_err(|e| format!("claude spawn: {e}"))
    }
}

/// Driver registry: name -> constructor. Adding a new harness = adding
/// one struct + one registry line.
pub fn lookup(name: &str) -> Option<Box<dyn HarnessDriver>> {
    match name {
        "omp" => Some(Box::new(OmpDriver)),
        "droid" => Some(Box::new(DroidDriver)),
        "pi" => Some(Box::new(PiDriver)),
        "claude" => Some(Box::new(ClaudeDriver)),
        _ => None,
    }
}

/// All known driver names, in registration order.
pub fn known() -> Vec<&'static str> {
    vec!["omp", "droid", "pi", "claude"]
}
