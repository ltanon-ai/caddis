//! play.rs — the KILLABLE PLAY CHILD, per device view (P3 slice b).
//!
//! Ports the daemon's proven playback pair (peluda_voice `audio.py` +
//! `play_proc.py`, the invariant sources named per behavior):
//!
//! - Playback runs in a SHORT-LIVED CHILD PROCESS, one per attempt. That
//!   makes an un-interruptible native wedge killable — the old in-process
//!   `sd.play` blocked the scheduler worker for 94 s and the watchdog's
//!   restart leaked the blocked thread. A cold child also gives a fresh
//!   audio stack every time, which removes the whole long-runtime
//!   degradation class.
//! - Health is truthful by construction: a named device is only ever
//!   played by EXACT name; the default sentinels play on the Windows
//!   default output device, resolved at open time in the fresh child —
//!   never a value cached at boot (operator order 2026-07-23: "always
//!   strictly the default sound card").
//! - The deadline is the audio's own duration PLUS the startup budget:
//!   an expired timeout killed a HEALTHY-BUT-SLOW child 106 times in the
//!   daemon's log before the budget was measured properly. The budget
//!   stays at the daemon's proven 15.0 s even though a Rust child's cold
//!   path is milliseconds — never tune a budget below the measured worst
//!   case without a new measurement.
//!
//! DIVERGENCE from the daemon, deliberate: the daemon fell through four
//! PortAudio host-API views (MME / DirectSound / WASAPI / WDM-KS) because
//! PortAudio exposed four personalities of the same hardware. The organ
//! child speaks winmm `waveOut` directly — the Windows-native API whose
//! PortAudio personality was "MME", the view the daemon tried FIRST as
//! "most permissive on this hardware". On Vista+ winmm is served by the
//! shared-mode audio engine, so rate conversion is the system's job; the
//! daemon's linear resample + channel fit are kept for the views that
//! refuse the native format (channel count especially — playing mono at
//! a 2-channel device is what produced PortAudio -9998).
//!
//! Child contract (exit codes mirror `play_proc.py` verbatim):
//! `caddis-voice play-view <wav> <device>`
//!   0  played            10  bad args / unreadable wav
//!   20 no matching view  30  stream open failed       40  play failed
//! On success the child prints ONE JSON line
//! `{"device","rate","channels","duration_s"}` (host_api is gone with
//! PortAudio; the line stays for humans and future telemetry).

use crate::transcribe::wav_meta;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Seconds allowed ON TOP of the audio's own duration before the child
/// is declared wedged (audio.py `PLAY_STARTUP_BUDGET_S`, verbatim).
pub const PLAY_STARTUP_BUDGET_S: f64 = 15.0;

/// The parent's verdict when the child had to be killed (audio.py
/// `TIMEOUT_RC`; a distinct value no honest child exit can collide with).
pub const TIMEOUT_RC: i32 = -9;

// play_proc.py exit codes, verbatim.
pub const EXIT_OK: i32 = 0;
pub const EXIT_BAD_INPUT: i32 = 10;
pub const EXIT_NO_VIEW: i32 = 20;
pub const EXIT_OPEN_FAILED: i32 = 30;
pub const EXIT_PLAY_FAILED: i32 = 40;

/// Poll cadence while waiting on children (piper.rs precedent: the cost
/// is a try_wait syscall, not a context switch storm).
const POLL_MS: u64 = 25;

/// When `device_name` is one of these, playback follows the Windows
/// DEFAULT output device (play_proc.py `DEFAULT_SENTINELS`, verbatim).
pub const DEFAULT_SENTINELS: [&str; 4] = ["", "default", "__default__", "system default"];

/// True when `device_name` asks for the system default output device.
pub fn is_default(device_name: &str) -> bool {
    DEFAULT_SENTINELS.contains(&device_name.trim().to_lowercase().as_str())
}

// ---------------------------------------------------------------------------
// PCM plumbing — play_proc.py's read/resample/fit, verbatim shapes
// ---------------------------------------------------------------------------

/// Interleaved f64 frames in [-1, 1] (the daemon's float32 domain; f64
/// because the arithmetic here is not hot-path enough to pay for f32
/// friction and the source of truth — the daemon — normalized anyway).
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub frames: Vec<f64>,
    pub channels: usize,
    pub rate: u32,
}

impl Pcm {
    fn sample(&self, frame: usize, ch: usize) -> f64 {
        self.frames[frame * self.channels + ch]
    }
}

/// Read a PCM16 WAV into [`Pcm`] (play_proc.py `read_wav`). Non-PCM16
/// input is bad input — the organ's renders and earcons are PCM16.
pub fn read_wav(bytes: &[u8]) -> Option<Pcm> {
    let meta = wav_meta(bytes)?;
    // wav_meta accepts float fmt too; the play child requires PCM16.
    let mut pos = 12;
    let mut fmt_bits: Option<u16> = None;
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        let body = pos + 8;
        if body + len > bytes.len() {
            return None;
        }
        match id {
            b"fmt " => {
                if len < 16 {
                    return None;
                }
                let audio_format =
                    u16::from_le_bytes([bytes[body], bytes[body + 1]]);
                if audio_format != 1 {
                    return None; // not PCM
                }
                fmt_bits = Some(u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]));
            }
            b"data" => data = Some(&bytes[body..body + len]),
            _ => {}
        }
        pos = body + len + (len & 1); // chunks are word-aligned
    }
    if fmt_bits != Some(16) {
        return None;
    }
    let data = data?;
    let n = data.len() / 2;
    let channels = meta.channels as usize;
    if channels == 0 || n % channels != 0 {
        return None;
    }
    let mut frames = Vec::with_capacity(n);
    for i in 0..n {
        let v = i16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
        frames.push(f64::from(v) / 32768.0);
    }
    Some(Pcm {
        frames,
        channels,
        rate: meta.sample_rate,
    })
}

/// Linear-interpolate to `dst_sr` (play_proc.py `resample` verbatim:
/// `n = len * dst / src`, sample points `linspace(0, len, n,
/// endpoint=False)` — position `i·src/dst`, never integer indices).
pub fn resample(pcm: &Pcm, dst_sr: u32) -> Pcm {
    if pcm.rate == dst_sr || pcm.frames.is_empty() {
        let mut out = pcm.clone();
        out.rate = dst_sr;
        return out;
    }
    let frames = pcm.frames.len() / pcm.channels;
    let n = frames * dst_sr as usize / pcm.rate as usize;
    let step = pcm.rate as f64 / dst_sr as f64;
    let mut out = Vec::with_capacity(n * pcm.channels);
    for i in 0..n {
        let pos = i as f64 * step;
        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(frames - 1);
        let frac = pos - i0 as f64;
        for ch in 0..pcm.channels {
            let a = pcm.sample(i0, ch);
            let b = pcm.sample(i1, ch);
            out.push(a + (b - a) * frac);
        }
    }
    Pcm {
        frames: out,
        channels: pcm.channels,
        rate: dst_sr,
    }
}

/// Up/down-mix to exactly `target` channels (play_proc.py
/// `fit_channels`, verbatim branch for branch: mono upmix repeats;
/// many-to-one averages; otherwise the FIRST channel is repeated).
pub fn fit_channels(pcm: &Pcm, target: usize) -> Pcm {
    let have = pcm.channels;
    if have == target || target == 0 {
        return pcm.clone();
    }
    let frames = pcm.frames.len() / have.max(1);
    let mut out = Vec::with_capacity(frames * target);
    for f in 0..frames {
        if have == 1 {
            let s = pcm.sample(f, 0);
            for _ in 0..target {
                out.push(s);
            }
        } else if target == 1 {
            let mut sum = 0.0;
            for ch in 0..have {
                sum += pcm.sample(f, ch);
            }
            out.push(sum / have as f64);
        } else {
            let s = pcm.sample(f, 0);
            for _ in 0..target {
                out.push(s);
            }
        }
    }
    Pcm {
        frames: out,
        channels: target,
        rate: pcm.rate,
    }
}

// ---------------------------------------------------------------------------
// winmm — raw FFI (std-only law, platform.rs precedent)
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod winmm {
    use std::time::Duration;
    // WAVE_MAPPER: the system default output device, resolved at open
    // time by winmm itself — exactly the operator's default-card ruling,
    // with no cached name to drift.
    pub const WAVE_MAPPER: usize = 0xFFFF_FFFF;
    pub const MMSYSERR_NOERROR: u32 = 0;
    const CALLBACK_NULL: u32 = 0;
    const WAVE_FORMAT_QUERY: u32 = 0x0000_0001;
    const WHDR_DONE: u32 = 0x0000_0001;

    #[repr(C)]
    struct WaveFormatEx {
        format_tag: u16,
        channels: u16,
        samples_per_sec: u32,
        avg_bytes_per_sec: u32,
        block_align: u16,
        bits_per_sample: u16,
        cb_size: u16,
    }

    #[repr(C)]
    struct WaveOutCapsW {
        manufacturer_id: u16,
        product_id: u16,
        driver_version: u32,
        pname: [u16; 32],
        formats: u32,
        channels: u16,
        reserved: [u16; 6],
    }

    #[repr(C)]
    struct WaveHdr {
        data: *mut u8,
        buffer_length: u32,
        bytes_recorded: u32,
        user: usize,
        flags: u32,
        loops: u32,
        next: usize,
        reserved: usize,
    }

    #[link(name = "winmm")]
    extern "system" {
        fn waveOutGetNumDevs() -> u32;
        fn waveOutGetDevCapsW(
            device_id: usize,
            caps: *mut WaveOutCapsW,
            caps_size: u32,
        ) -> u32;
        fn waveOutOpen(
            handle: *mut usize,
            device_id: usize,
            format: *const WaveFormatEx,
            callback: usize,
            instance: usize,
            flags: u32,
        ) -> u32;
        fn waveOutPrepareHeader(handle: usize, hdr: *mut WaveHdr, size: u32) -> u32;
        fn waveOutWrite(handle: usize, hdr: *mut WaveHdr, size: u32) -> u32;
        fn waveOutUnprepareHeader(handle: usize, hdr: *mut WaveHdr, size: u32) -> u32;
        fn waveOutClose(handle: usize) -> u32;
    }

    fn from_utf16z(buf: &[u16; 32]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    fn fmt_(channels: u16, rate: u32, bits: u16) -> WaveFormatEx {
        let block = channels * bits / 8;
        WaveFormatEx {
            format_tag: 1, // PCM
            channels,
            samples_per_sec: rate,
            avg_bytes_per_sec: rate * u32::from(block),
            block_align: block,
            bits_per_sample: bits,
            cb_size: 0,
        }
    }

    /// The exact-name view (health truth law: only an exact-name device
    /// is ever played). `None` = no such device.
    pub fn find_named(device_name: &str) -> Option<usize> {
        let n = unsafe { waveOutGetNumDevs() };
        let mut caps = WaveOutCapsW {
            manufacturer_id: 0,
            product_id: 0,
            driver_version: 0,
            pname: [0; 32],
            formats: 0,
            channels: 0,
            reserved: [0; 6],
        };
        (0..n as usize).find(|&id| {
            let ok = unsafe {
                waveOutGetDevCapsW(id, &mut caps, std::mem::size_of::<WaveOutCapsW>() as u32)
            } == MMSYSERR_NOERROR;
            ok && from_utf16z(&caps.pname) == device_name
        })
    }

    /// Play interleaved f64 frames as PCM16 on the chosen device id.
    /// Returns Err(open) / Err(play) mapped by the caller to 30/40.
    pub fn play_on(device_id: usize, src: &super::Pcm) -> Result<(), super::PlayFail> {
        let query = |ch: u16, rate: u32| -> u32 {
            let f = fmt_(ch, rate, 16);
            let mut h = WAVE_MAPPER;
            unsafe {
                waveOutOpen(&mut h, device_id, &f, 0, 0, CALLBACK_NULL | WAVE_FORMAT_QUERY)
            }
        };
        // Format probe first (play_proc checked output settings): a view
        // that refuses the native shape gets one adapted retry at the
        // universal MME format — 44100 stereo PCM16 (channel count
        // especially: mono at a 2-channel view is what produced
        // PortAudio -9998).
        let native_ch = u16::try_from(src.channels).unwrap_or(2);
        let adapted;
        let (pcm, channels, rate) = if query(native_ch, src.rate) == MMSYSERR_NOERROR {
            (src, native_ch, src.rate)
        } else {
            adapted = super::fit_channels(&super::resample(src, 44_100), 2);
            let alt_rc = query(2, 44_100);
            if alt_rc != MMSYSERR_NOERROR {
                return Err(super::PlayFail::Open(alt_rc));
            }
            (&adapted, 2, 44_100)
        };

        // PCM16 bytes.
        let mut bytes = Vec::with_capacity(pcm.frames.len() * 2);
        for s in &pcm.frames {
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            bytes.extend_from_slice(&v.to_le_bytes());
        }

        let mut handle = WAVE_MAPPER;
        let format = fmt_(channels, rate, 16);
        let rc = unsafe { waveOutOpen(&mut handle, device_id, &format, 0, 0, CALLBACK_NULL) };
        if rc != MMSYSERR_NOERROR {
            return Err(super::PlayFail::Open(rc));
        }
        let finish = |handle: usize| unsafe {
            waveOutClose(handle);
        };
        let mut hdr = WaveHdr {
            data: bytes.as_mut_ptr(),
            buffer_length: bytes.len() as u32,
            bytes_recorded: 0,
            user: 0,
            flags: 0,
            loops: 0,
            next: 0,
            reserved: 0,
        };
        let hdr_size = std::mem::size_of::<WaveHdr>() as u32;
        let prc = unsafe { waveOutPrepareHeader(handle, &mut hdr, hdr_size) };
        if prc != MMSYSERR_NOERROR {
            finish(handle);
            return Err(super::PlayFail::Play(prc));
        }
        let wrc = unsafe { waveOutWrite(handle, &mut hdr, hdr_size) };
        if wrc != MMSYSERR_NOERROR {
            finish(handle);
            return Err(super::PlayFail::Play(wrc));
        }
        // Synchronous wait on the header's DONE flag (the daemon child
        // used sd.play(blocking=True); the parent's deadline is the wedge
        // backstop, this loop only paces).
        loop {
            if hdr.flags & WHDR_DONE != 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(super::POLL_MS));
        }
        unsafe { waveOutUnprepareHeader(handle, &mut hdr, hdr_size) };
        finish(handle);
        Ok(())
    }
}


/// winmm failure phase (maps to exit 30 / 40).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayFail {
    Open(u32),
    Play(u32),
}

/// THE CHILD: play one wav on one device view. Returns the process exit
/// code (the `play-view` contract; `main.rs` is the thin argv wrapper).
pub fn play_view(wav_path: &str, device_name: &str) -> i32 {
    let bytes = match std::fs::read(wav_path) {
        Ok(b) => b,
        Err(_) => return EXIT_BAD_INPUT,
    };
    let pcm = match read_wav(&bytes) {
        Some(p) => p,
        None => return EXIT_BAD_INPUT,
    };
    #[cfg(windows)]
    {
        let id = if is_default(device_name) {
            winmm::WAVE_MAPPER
        } else {
            // Health truth law: only an EXACT-NAME device is ever played.
            match winmm::find_named(device_name) {
                Some(id) => id,
                None => return EXIT_NO_VIEW,
            }
        };
        match winmm::play_on(id, &pcm) {
            Ok(()) => {
                let duration =
                    pcm.frames.len() as f64 / (pcm.channels * pcm.rate as usize) as f64;
                println!(
                    "{{\"device\":\"{}\",\"rate\":{},\"channels\":{},\"duration_s\":{:.3}}}",
                    if is_default(device_name) { "default" } else { device_name },
                    pcm.rate,
                    pcm.channels,
                    duration
                );
                EXIT_OK
            }
            Err(PlayFail::Open(rc)) => {
                eprintln!("play-view: waveOutOpen failed rc={rc}");
                EXIT_OPEN_FAILED
            }
            Err(PlayFail::Play(rc)) => {
                eprintln!("play-view: waveOut play failed rc={rc}");
                EXIT_PLAY_FAILED
            }
        }
    }
    #[cfg(not(windows))]
    {
        // Honest unsupported (job.rs law): the organ's supervision design
        // is a Windows law; pretending a view exists would fake health.
        let _ = (&pcm, device_name);
        eprintln!("play-view: playback needs winmm (windows)");
        EXIT_OPEN_FAILED
    }
}

// ---------------------------------------------------------------------------
// AudioOut — the parent-side arbiter (audio.py port)
// ---------------------------------------------------------------------------

/// The parent's verdict for one playback (audio.py `PlaybackOutcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackOutcome {
    Played,
    AudioFailed,
    Timeout,
}

impl PlaybackOutcome {
    pub fn ok(&self) -> bool {
        matches!(self, PlaybackOutcome::Played)
    }
}


/// Plays wavs through the named device, one killable child per attempt
/// (audio.py `AudioOut`). Single-owner by design: the dispatch worker
/// owns it, exactly like the daemon's worker thread owned the audio
/// lock; the daemon's internal lock existed for tray/IPC callers this
/// organ does not have.
#[derive(Debug, Clone)]
pub struct AudioOut {
    device_name: String,
    startup_budget_s: f64,
    /// TEST/STUB lane only (piper.rs argv precedent). `{WAV}` and
    /// `{DEVICE}` are substituted per attempt. Production spawns the
    /// organ's own executable via `current_exe()`.
    child_argv: Option<Vec<String>>,
    last_error: Option<String>,
    error_state: bool,
    /// Pid of the child currently inside `spawn_and_wait` (kill_child's
    /// target; None between attempts). Owner-held, not a lock: the
    /// dispatch worker owns this struct.
    in_flight: Option<u32>,
}

impl AudioOut {
    pub fn new(device_name: &str) -> Self {
        AudioOut {
            device_name: device_name.to_string(),
            startup_budget_s: PLAY_STARTUP_BUDGET_S,
            child_argv: None,
            last_error: None,
            error_state: false,
            in_flight: None,
        }
    }

    /// TEST LANE ONLY: replace the child argv and shrink the budget.
    pub fn with_child_argv(mut self, argv: Vec<String>, startup_budget_s: f64) -> Self {
        self.child_argv = Some(argv);
        self.startup_budget_s = startup_budget_s;
        self
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn error_state(&self) -> bool {
        self.error_state
    }

    fn argv(&self, wav: &str) -> Vec<String> {
        match &self.child_argv {
            Some(ov) => ov
                .iter()
                .map(|a| a.replace("{WAV}", wav).replace("{DEVICE}", &self.device_name))
                .collect(),
            None => {
                let exe = std::env::current_exe()
                    .map(|p: PathBuf| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                vec![exe, "play-view".into(), wav.into(), self.device_name.clone()]
            }
        }
    }

    /// Play one wav through the first view that works. The deadline is
    /// the audio's own duration plus the startup budget (audio.py
    /// `play`); a wedged child is KILLED — that killability is the
    /// entire reason the child exists.
    pub fn play(&mut self, wav: &[u8]) -> PlaybackOutcome {
        let duration_s = wav_meta(wav).map(|m| m.duration_s).unwrap_or(0.0);
        let deadline = duration_s + self.startup_budget_s;

        let path = temp_wav();
        if let Err(e) = std::fs::write(&path, wav) {
            self.fail(format!("write temp wav failed: {e}"));
            return PlaybackOutcome::AudioFailed;
        }
        let wav = path.to_string_lossy();
        let outcome = self.spawn_and_wait(&wav, deadline);
        let _ = std::fs::remove_file(&path);
        match outcome {
            Some(EXIT_OK) => {
                self.last_error = None;
                self.error_state = false;
                PlaybackOutcome::Played
            }
            Some(TIMEOUT_RC) => {
                self.fail(format!(
                    "play child wedged on device {:?} after {deadline:.1}s",
                    self.device_name
                ));
                PlaybackOutcome::Timeout
            }
            Some(rc) => {
                self.fail(format!("play failed on device {:?} (rc={rc})", self.device_name));
                PlaybackOutcome::AudioFailed
            }
            None => {
                self.fail(format!("play child spawn failed for {:?}", self.device_name));
                PlaybackOutcome::AudioFailed
            }
        }
    }

    /// Kill any in-flight child (worker restart / shutdown —
    /// audio.py `kill_child`).
    pub fn kill_child(&mut self) {
        if let Some(pid) = self.in_flight.take() {
            kill_pid(pid);
        }
    }

    fn fail(&mut self, msg: String) {
        self.last_error = Some(msg.clone());
        self.error_state = true;
    }

    fn spawn_and_wait(&mut self, wav_path: &str, deadline_s: f64) -> Option<i32> {
        let argv = self.argv(wav_path);
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        // stdout piped and DRAINED (the child's JSON line; an undrained
        // pipe deadlocks); stderr null.
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().ok()?;
        let pid = child.id();
        self.in_flight = Some(pid);
        let started = Instant::now();
        let mut stdout = child.stdout.take();
        let mut drain = [0u8; 512];
        loop {
            match child.try_wait() {
                Ok(Some(st)) => {
                    // Drain what is left so the child never blocked on us.
                    if let Some(out) = stdout.as_mut() {
                        let _ = out.read(&mut drain);
                    }
                    self.in_flight = None;
                    return Some(st.code().unwrap_or(EXIT_PLAY_FAILED));
                }
                Ok(None) => {}
                Err(_) => {
                    self.in_flight = None;
                    return Some(EXIT_PLAY_FAILED);
                }
            }
            if let Some(out) = stdout.as_mut() {
                let _ = out.read(&mut drain);
            }
            if started.elapsed().as_secs_f64() > deadline_s {
                let _ = child.kill();
                let _ = child.wait();
                self.in_flight = None;
                return Some(TIMEOUT_RC);
            }
            std::thread::sleep(Duration::from_millis(POLL_MS));
        }
    }
}

#[cfg(windows)]
fn kill_pid(pid: u32) {
    // std has no cross-process kill by id; taskkill is the Windows-native
    // scoped kill (the child is ours alone by construction).
    use std::os::windows::process::CommandExt;
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F", "/T"])
        .creation_flags(0x0800_0000)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(windows))]
fn kill_pid(_pid: u32) {}

/// Unique temp wav for one attempt (piper.rs temp_pair precedent:
/// nanos + per-process counter + pid).
fn temp_wav() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = format!(
        "caddisplay_{}_{}_{nanos}.wav",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );
    std::env::temp_dir().join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal PCM16 wav: `secs` of near-silence at `rate`/`ch`.
    fn tiny_wav(secs: f64, rate: u32, ch: u16) -> Vec<u8> {
        let frames = (secs * f64::from(rate)).round() as usize;
        let data_len = frames * ch as usize * 2;
        let byte_rate = rate * u32::from(ch) * 2;
        let block_align = ch * 2;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
        b.extend_from_slice(b"WAVE");
        b.extend_from_slice(b"fmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&ch.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&byte_rate.to_le_bytes());
        b.extend_from_slice(&block_align.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data_len as u32).to_le_bytes());
        // tiny nonzero ramp so resample has signal
        for i in 0..frames {
            let v = ((i % 100) as f64 / 100.0 * 2.0 - 1.0) * 8000.0;
            for _ in 0..ch {
                b.extend_from_slice(&(v as i16).to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn default_sentinels_verbatim() {
        for s in ["", "default", "__default__", "system default", "DEFAULT", " Default "] {
            assert!(is_default(s), "{s:?} must be a default sentinel");
        }
        assert!(!is_default(" Speakers (AMD HD Audio) "));
    }

    #[test]
    fn read_wav_roundtrip_and_rejects() {
        let wav = tiny_wav(0.5, 22_050, 1);
        let pcm = read_wav(&wav).expect("pcm16 wav parses");
        assert_eq!((pcm.rate, pcm.channels), (22_050, 1));
        assert_eq!(pcm.frames.len(), 11_025);
        assert!(pcm.frames.iter().all(|s| (-1.0..=1.0).contains(s)));
        assert!(read_wav(b"not a wav").is_none());
        assert!(read_wav(&[]).is_none());
    }

    #[test]
    fn resample_doubles_length_and_keeps_bounds() {
        let wav = tiny_wav(0.25, 8_000, 1);
        let pcm = read_wav(&wav).unwrap();
        let out = resample(&pcm, 16_000);
        assert_eq!(out.rate, 16_000);
        assert_eq!(out.frames.len(), pcm.frames.len() * 2);
        // endpoints preserved by interpolation domain
        assert!((out.frames[0] - pcm.frames[0]).abs() < 1e-9);
        assert!(out.frames.iter().all(|s| (-1.0..=1.0).contains(s)));
    }

    #[test]
    fn fit_channels_all_branches_verbatim() {
        let mono = Pcm {
            frames: vec![0.5, -0.25, 0.75],
            channels: 1,
            rate: 16_000,
        };
        let stereo = fit_channels(&mono, 2);
        assert_eq!(stereo.channels, 2);
        assert_eq!(stereo.frames, vec![0.5, 0.5, -0.25, -0.25, 0.75, 0.75]);

        let back = fit_channels(&stereo, 1);
        assert_eq!(back.channels, 1);
        assert_eq!(back.frames, vec![0.5, -0.25, 0.75]); // mean of equal pair

        let tri = Pcm {
            frames: vec![0.1, 0.9, 0.2, 0.8],
            channels: 2,
            rate: 16_000,
        };
        let tri3 = fit_channels(&tri, 3);
        assert_eq!(tri3.channels, 3);
        // first channel repeated (daemon's final branch)
        assert_eq!(tri3.frames, vec![0.1, 0.1, 0.1, 0.2, 0.2, 0.2]);
    }

    #[test]
    #[cfg(windows)]
    fn audioout_failed_child_maps_to_audio_failed() {
        // cmd exits 3 immediately: parent must report AudioFailed with a
        // truthful last_error, never hang, never timeout.
        let mut out = AudioOut::new("default").with_child_argv(
            vec!["cmd".into(), "/c".into(), "exit".into(), "3".into()],
            5.0,
        );
        let wav = tiny_wav(0.05, 22_050, 1);
        assert_eq!(out.play(&wav), PlaybackOutcome::AudioFailed);
        assert!(out.error_state());
        assert!(out.last_error().unwrap().contains("rc=3"));
    }

    #[test]
    #[cfg(windows)]
    fn audioout_wedged_child_is_killed() {
        // ping sleeps ~30s; the deadline (0.05s audio + 0.4s budget) must
        // kill it and report Timeout.
        let mut out = AudioOut::new("default").with_child_argv(
            vec!["ping".into(), "-n".into(), "30".into(), "127.0.0.1".into()],
            0.4,
        );
        let wav = tiny_wav(0.05, 22_050, 1);
        let started = Instant::now();
        assert_eq!(out.play(&wav), PlaybackOutcome::Timeout);
        assert!(started.elapsed().as_secs() < 10, "child must die fast");
        assert!(out.last_error().unwrap().contains("wedged"));
    }

    #[test]
    #[cfg(windows)]
    fn audioout_spawn_failure_is_audio_failed() {
        let mut out = AudioOut::new("default").with_child_argv(
            vec!["no-such-exe-xyz".into()],
            5.0,
        );
        let wav = tiny_wav(0.05, 22_050, 1);
        assert_eq!(out.play(&wav), PlaybackOutcome::AudioFailed);
        assert!(out.last_error().unwrap().contains("spawn failed"));
    }

    #[test]
    fn timeout_rc_cannot_collide_with_honest_exits() {
        for honest in [EXIT_OK, EXIT_BAD_INPUT, EXIT_NO_VIEW, EXIT_OPEN_FAILED, EXIT_PLAY_FAILED]
        {
            assert_ne!(honest, TIMEOUT_RC);
        }
    }
}
