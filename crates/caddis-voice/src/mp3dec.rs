//! mp3dec.rs — the network lane's MP3→WAV decode child.
//!
//! The edge endpoint refuses every uncompressed output format (live
//! sweep 2026-08-27: `raw-*` and `riff-*` close the stream; only
//! `*-mp3` and `webm-opus` complete) — so the network lane decodes its
//! MP3 wire format to RIFF/WAVE before the render is returned. Decode
//! runs in an operator-configured CLI child (the piper.exe precedent:
//! exe path in config, the organ owns the contract):
//!
//! `mp3_decoder_exe -hide_banner -loglevel error -y -i <in.mp3> -f wav
//! -acodec pcm_s16le -ar 24000 -ac 1 <out.wav>`
//!
//! Same shape as a piper render: temp files (never pipes — a pipe
//! deadlocks the moment either side outgrows the kernel buffer), the
//! [`ChildScope`] job so a wedged child dies with us, a flat kill
//! deadline, and GA2 WAV validation before anyone hears a sample.

use crate::adapter::{validate_wav, AdapterErr};
use crate::job::ChildScope;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// Flat kill deadline for one decode (generous against the ~100 ms a
/// speech clip takes; bounded against a wedged decoder).
const KILL_DEADLINE_MS: u128 = 5_000;
const POLL_MS: u64 = 25;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Decode `mp3` to 24 kHz mono 16-bit WAV via the configured exe.
/// Returns GA2-validated WAV bytes.
pub fn decode_mp3_to_wav(exe: &str, mp3: &[u8]) -> Result<Vec<u8>, AdapterErr> {
    let started = std::time::Instant::now();
    let (in_path, out_path) = temp_paths();
    std::fs::write(&in_path, mp3).map_err(|e| AdapterErr(format!("mp3dec: write input: {e}")))?;

    let mut cmd = Command::new(exe);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&in_path)
        .args([
            "-f",
            "wav",
            "-acodec",
            "pcm_s16le",
            "-ar",
            "24000",
            "-ac",
            "1",
        ])
        .arg(&out_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| {
        let _ = std::fs::remove_file(&in_path);
        AdapterErr(format!("mp3dec: spawn {exe}: {e}"))
    })?;
    let pid = child.id();

    // QQ2/F2: the job kills this child if we die mid-decode; on the
    // timeout path, dropping the scope is the kill.
    let scope = ChildScope::create().map_err(|e| AdapterErr(format!("mp3dec: job scope: {e}")));
    if let Ok(s) = &scope {
        if let Err(e) = s.assign_pid(pid) {
            let _ = std::fs::remove_file(&in_path);
            return Err(AdapterErr(format!("mp3dec: job assign: {e}")));
        }
    }

    let st = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {}
            Err(e) => {
                drop(scope);
                let _ = std::fs::remove_file(&in_path);
                let _ = std::fs::remove_file(&out_path);
                return Err(AdapterErr(format!("mp3dec: wait failed: {e}")));
            }
        }
        if started.elapsed().as_millis() > KILL_DEADLINE_MS {
            drop(scope);
            let _ = child.kill();
            let _ = std::fs::remove_file(&in_path);
            let _ = std::fs::remove_file(&out_path);
            return Err(AdapterErr(format!(
                "mp3dec: decode exceeded {}ms kill deadline",
                KILL_DEADLINE_MS
            )));
        }
        std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
    };
    drop(scope);
    let _ = std::fs::remove_file(&in_path);

    if !st.success() {
        let _ = std::fs::remove_file(&out_path);
        return Err(AdapterErr(format!("mp3dec: exited {st}")));
    }
    let bytes = match std::fs::read(&out_path) {
        Ok(b) => b,
        Err(e) => {
            let _ = std::fs::remove_file(&out_path);
            return Err(AdapterErr(format!("mp3dec: no output wav: {e}")));
        }
    };
    let _ = std::fs::remove_file(&out_path);
    validate_wav(&bytes)?;
    Ok(bytes)
}

fn temp_paths() -> (PathBuf, PathBuf) {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir();
    (
        dir.join(format!("caddis-mp3dec-{n}.mp3")),
        dir.join(format!("caddis-mp3dec-{n}.wav")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The operator-box ffmpeg (also the config's default on this
    /// machine). Absent → these tests skip, never fail: the child is an
    /// operator-supplied deployment fact, not a repo invariant.
    const FFMPEG: &str = "C:/ffmpeg/bin/ffmpeg.exe";

    fn have_ffmpeg() -> bool {
        std::path::Path::new(FFMPEG).exists()
    }

    /// Make a real ~0.6 s MP3 with ffmpeg itself (a 440 Hz sine): the
    /// decoder's contract is real MP3, not a fixture stand-in.
    fn make_mp3() -> Option<Vec<u8>> {
        if !have_ffmpeg() {
            return None;
        }
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let out_path = std::env::temp_dir().join(format!("caddis-mp3dec-fx-{n}.mp3"));
        let st = Command::new(FFMPEG)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .arg("-i")
            .arg("sine=frequency=440:duration=0.6")
            .args(["-acodec", "libmp3lame", "-b:a", "48k"])
            .arg(&out_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let bytes = st
            .ok()
            .filter(|s| s.success())
            .and_then(|_| std::fs::read(&out_path).ok());
        let _ = std::fs::remove_file(&out_path);
        bytes
    }

    #[test]
    fn decodes_real_mp3_to_valid_wav() {
        let Some(mp3) = make_mp3() else {
            eprintln!("skip: no ffmpeg at {FFMPEG}");
            return;
        };
        let wav = decode_mp3_to_wav(FFMPEG, &mp3).expect("decode");
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        // 24 kHz mono 16-bit — the args pin it; the header proves it.
        let rate = u32::from_le_bytes(wav[24..28].try_into().unwrap());
        let channels = u16::from_le_bytes(wav[22..24].try_into().unwrap());
        assert_eq!((rate, channels), (24_000, 1));
        // ~0.6 s of audio ± tolerance.
        let meta = crate::transcribe::wav_meta(&wav).expect("meta");
        assert!(
            meta.duration_s > 0.3 && meta.duration_s < 1.2,
            "{}",
            meta.duration_s
        );
    }

    #[test]
    fn garbage_input_fails_closed_not_garbled() {
        if !have_ffmpeg() {
            eprintln!("skip: no ffmpeg at {FFMPEG}");
            return;
        }
        let e = decode_mp3_to_wav(FFMPEG, &[0x00u8; 2_000]).unwrap_err();
        // Exit-status or GA2 rejection — never a half-decoded payload.
        assert!(
            e.0.contains("exited") || e.0.contains("GA2"),
            "unexpected err: {e}"
        );
    }

    #[test]
    fn missing_exe_is_a_clean_spawn_error() {
        let e =
            decode_mp3_to_wav("Z:/definitely/not/here.exe", &[0xFF, 0xFB, 0x00, 0x00]).unwrap_err();
        assert!(e.0.contains("spawn"), "unexpected err: {e}");
    }
}
