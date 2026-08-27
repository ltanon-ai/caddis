//! vram.rs — the VRAM capacity report (QQ2: /health carries it BEFORE any
//! engine spawn).
//!
//! Why it exists: an engine (piper with GPU offload, a whisper Vulkan server)
//! that spawns into an exhausted GPU fails in the WORST way — slowly, mid-
//! utterance, or by destabilizing a neighbor lane. QQ2's ruling makes the
//! capacity visible at /health from P1, before the first spawn call can
//! exist (P2). The spawn-side refusal ("vram snapshot is stale → take one
//! first") lands with the engines; the measurement surface lands here.
//!
//! Windows path: dxgi.dll's exported `CreateDXGIFactory1` (no COM init, no
//! CoCreateInstance — the direct export does its own), `EnumAdapters1`,
//! `GetDesc1`. Raw `extern "system"` declarations, std-only law, exactly like
//! caddis-memory's winprobe (no windows-sys dependency).
//!
//! Failure doctrine: FAILOPEN-REPORT — a probe failure never blocks the organ
//! (capacity reporting is telemetry, not a gate by itself); the report says
//! `source: "unavailable"` + reason, and the health endpoint stays truthful.
//! Health may say "unknown"; it may never invent a number.

use crate::json::Value;

/// One adapter's memory facts, as DXGI reports them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterMem {
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    /// VRAM dedicated to the adapter (the number spawn decisions want).
    pub dedicated_video_bytes: u64,
    /// Dedicated system memory DXGI carves for the adapter.
    pub dedicated_system_bytes: u64,
    /// Shared system memory the adapter may also use.
    pub shared_bytes: u64,
}

/// The whole probe result. `source` names the measurement lane so a consumer
/// can tell "measured" from "could not measure".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VramReport {
    pub source: &'static str,
    pub reason: Option<String>,
    pub adapters: Vec<AdapterMem>,
}

impl VramReport {
    /// Sum of dedicated video memory across adapters. 0 when unknown —
    /// pair with `source` before trusting it.
    pub fn total_dedicated_video_bytes(&self) -> u64 {
        self.adapters.iter().map(|a| a.dedicated_video_bytes).sum()
    }

    /// JSON shape served by /health (field order is the readable contract).
    pub fn to_value(&self) -> Value {
        let mut adapters = Vec::new();
        for a in &self.adapters {
            adapters.push(Value::Obj(vec![
                ("name".into(), Value::Str(a.name.clone())),
                ("vendor_id".into(), Value::Num(a.vendor_id as f64)),
                ("device_id".into(), Value::Num(a.device_id as f64)),
                (
                    "dedicated_video_bytes".into(),
                    Value::Num(a.dedicated_video_bytes as f64),
                ),
                (
                    "dedicated_system_bytes".into(),
                    Value::Num(a.dedicated_system_bytes as f64),
                ),
                ("shared_bytes".into(), Value::Num(a.shared_bytes as f64)),
            ]));
        }
        Value::Obj(vec![
            ("source".into(), Value::Str(self.source.into())),
            (
                "reason".into(),
                self.reason.clone().map(Value::Str).unwrap_or(Value::Null),
            ),
            ("adapters".into(), Value::Arr(adapters)),
            (
                "total_dedicated_video_bytes".into(),
                Value::Num(self.total_dedicated_video_bytes() as f64),
            ),
        ])
    }
}

/// Probe adapter memory now. Never panics; never blocks.
pub fn probe() -> VramReport {
    match crate::platform::vram_probe() {
        Ok(a) => VramReport {
            source: "dxgi",
            reason: None,
            adapters: a,
        },
        Err(reason) => VramReport {
            source: "unavailable",
            reason: Some(reason),
            adapters: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_on_this_host_reports_adapters_or_honest_unknown() {
        let r = probe();
        // On the organ host (Windows + real GPU) DXGI must enumerate; on a
        // machine where it cannot, the report says so instead of inventing.
        if r.source == "dxgi" {
            assert!(
                !r.adapters.is_empty(),
                "dxgi source with zero adapters is a parse bug"
            );
            assert!(r.total_dedicated_video_bytes() > 0);
            assert!(r.adapters.iter().all(|a| !a.name.is_empty()));
        } else {
            assert!(r.reason.is_some(), "unavailable must carry a reason");
        }
    }

    #[test]
    fn json_shape_is_the_health_contract() {
        let r = VramReport {
            source: "unavailable",
            reason: Some("no dxgi".into()),
            adapters: vec![],
        };
        let s = crate::json::to_string(&r.to_value());
        assert!(s.contains(r#""source":"unavailable""#), "{s}");
        assert!(s.contains(r#""total_dedicated_video_bytes":0"#), "{s}");
        let measured = VramReport {
            source: "dxgi",
            reason: None,
            adapters: vec![AdapterMem {
                name: "Test GPU".into(),
                vendor_id: 0x1002,
                device_id: 0x7bf1,
                dedicated_video_bytes: 48,
                dedicated_system_bytes: 0,
                shared_bytes: 16,
            }],
        };
        let v = measured.to_value();
        assert_eq!(
            v.get("adapters").and_then(Value::as_arr).map(|a| a.len()),
            Some(1)
        );
        assert!(crate::json::to_string(&v).contains(r#""name":"Test GPU""#));
    }
}
