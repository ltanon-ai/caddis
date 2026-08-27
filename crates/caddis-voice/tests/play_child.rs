//! Integration: the REAL `play-view` child binary (P3 slice b).
//!
//! Exercises the exit-code contract of `caddis-voice play-view`
//! (play_proc.py port) WITHOUT touching any audio device:
//!   10 bad input (missing / non-PCM16 wav)
//!   20 no matching view (exact-name device that cannot exist)
//!   2  usage (wrong argc)
//! Playing real audio is the P3 in-situ drill — the operator listens,
//! a CI box must not.

#[cfg(windows)]
mod win {
    use std::io::Write;
    use std::process::Command;

    fn bin() -> Command {
        Command::new(env!("CARGO_BIN_EXE_caddis-voice"))
    }

    fn tiny_wav(path: &std::path::Path) {
        let rate = 22_050u32;
        let frames = 2205usize; // 0.1 s mono
        let data_len = frames * 2;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
        b.extend_from_slice(b"WAVE");
        b.extend_from_slice(b"fmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data_len as u32).to_le_bytes());
        b.extend(std::iter::repeat_n([0u8, 0], frames).flatten());
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(&b).unwrap();
    }

    #[test]
    fn missing_wav_is_exit_10() {
        let st = bin()
            .args([
                "play-view",
                "Z:/no/such/dir/x.wav",
                "no-such-device-caddis-test",
            ])
            .output()
            .unwrap();
        assert_eq!(st.status.code(), Some(10), "unreadable wav => EXIT_BAD_INPUT");
    }

    #[test]
    fn non_pcm16_wav_is_exit_10() {
        let dir = std::env::temp_dir().join("caddis-playchild-it");
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("bad.wav");
        std::fs::write(&wav, b"RIFF----WAVEjunk").unwrap();
        let st = bin()
            .args(["play-view", wav.to_str().unwrap(), "no-such-device-caddis-test"])
            .output()
            .unwrap();
        assert_eq!(st.status.code(), Some(10), "non-PCM16 => EXIT_BAD_INPUT");
    }

    #[test]
    fn unknown_exact_name_device_is_exit_20() {
        let dir = std::env::temp_dir().join("caddis-playchild-it");
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("ok.wav");
        tiny_wav(&wav);
        // A wav that PARSES but a device name that cannot exist: the
        // health-truth law says unknown name is NO_VIEW, never a default
        // fallback.
        let st = bin()
            .args(["play-view", wav.to_str().unwrap(), "no-such-device-caddis-test"])
            .output()
            .unwrap();
        assert_eq!(
            st.status.code(),
            Some(20),
            "exact-name miss => EXIT_NO_VIEW (never a silent default)"
        );
    }

    #[test]
    fn wrong_argc_is_usage_exit_2() {
        let st = bin().arg("play-view").output().unwrap();
        assert_eq!(st.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&st.stderr);
        assert!(stderr.contains("usage"), "fail closed with usage: {stderr}");
    }
}
