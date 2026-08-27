//! LIVE probe: a real offline render through the piper adapter against
//! the DEPLOYED engine (read-only: reads piper.exe + the amy ONNX model
//! the peluda daemon serves from; writes only its own temp render files).
//!
//! NOT part of the default suite (engine + model are machine-local,
//! renders cost 1.6-4.6s): run explicitly with
//! `cargo test -p caddis-voice --test live_piper -- --ignored --nocapture`.

use caddis_voice::lang::Lang;
use caddis_voice::registry::VoiceSpec;
use caddis_voice::{AudioFormat, PiperAdapter, PiperPaths, PIPER_KILL_DEADLINE_MS};

const PIPER_EXE: &str =
    "C:\\Users\\ashpac\\AppData\\Local\\Programs\\Python\\Python310\\Scripts\\piper.exe";
const PIPER_MODEL: &str =
    "C:\\Users\\ashpac\\.pi\\agent\\peluda-voice\\peluda_voice\\voices\\en_US-amy-medium.onnx";

#[test]
#[ignore]
fn live_piper_render_end_to_end() {
    let adapter = PiperAdapter::new(
        PiperPaths {
            exe: PIPER_EXE.into(),
            model: PIPER_MODEL.into(),
            model_config: None,
        },
        1500,
    );

    let voice = VoiceSpec {
        id: "en_US-amy".into(),
        generator: "piper".into(),
        lang: Lang::En,
    };
    let started = std::time::Instant::now();
    let r = adapter
        .render(
            &voice,
            "Voice organ adapter live probe. Piper lane speaking.",
            1.0,
        )
        .expect("live render must succeed");
    let wall = started.elapsed();

    println!(
        "live piper: {} bytes WAV, {} ms wall (in-render {} ms, over_cap={})",
        r.bytes.len(),
        wall.as_millis(),
        r.elapsed_ms,
        r.over_cap
    );
    assert_eq!(r.format, AudioFormat::Wav);
    assert_eq!(r.voice, "en_US-amy");
    // GA2 offline validation already ran inside render(); sanity: header.
    assert_eq!(&r.bytes[0..4], b"RIFF");
    // A real spoken sentence is at least a half-second of audio.
    assert!(
        r.bytes.len() > 22_050,
        "implausibly small WAV: {}",
        r.bytes.len()
    );
    // And it must land under the proven kill budget.
    assert!(r.elapsed_ms < PIPER_KILL_DEADLINE_MS);
}
