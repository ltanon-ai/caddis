//! idempotency.rs — atmintinis idem_key rinkinys (CARD-0001 step 4); pakartojimas -> Err(E-IDEM).
use std::collections::HashSet;

#[derive(Default)]
pub struct Idempotency {
    seen: HashSet<String>,
}

impl Idempotency {
    pub fn new() -> Self {
        Self {
            seen: HashSet::new(),
        }
    }
    pub fn check(&mut self, key: &str) -> Result<(), &'static str> {
        if self.seen.contains(key) {
            return Err("E-IDEM: duplicate idem_key");
        }
        self.seen.insert(key.to_string());
        Ok(())
    }
    pub fn reset(&mut self) {
        self.seen.clear();
    }
}
