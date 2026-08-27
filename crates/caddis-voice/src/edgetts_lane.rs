//! edgetts_lane.rs — the RenderLane adapter for the network LT
//! generators (leonas + ona; T-35 P4 slice d).
//!
//! One lane per GENERATOR, not per voice: the dispatcher keys lanes by
//! generator id and the registry binds voices to generators — the voice
//! id IS the edge-tts voice name ("lt-LT-LeonasNeural"), identity by
//! construction.
//!
//! Every render: fresh DRM token, fresh connection id (attempt-counter
//! seeded — a connection id is never reused), GA1-authorized dial, and
//! the R-D single-attempt budget (`lt_network_deadline_ms`) covering
//! dial + exchange. No retries by law (R-D): one attempt, one verdict;
//! sustained failure is the GA3 breaker's business, not this lane's.
//! `length_scale` is piper's knob — the edge lane shapes speech only
//! through prosody, so it is deliberately ignored.

use crate::adapter::{AdapterErr, RenderedAudio};
use crate::edgetts::{dial_url, hex_id32, sec_ms_gec, synthesize, Prosody, SessionOpts, WsStream};
use crate::registry::{GeneratorSpec, VoiceSpec};
use crate::say::RenderLane;
use crate::wss::WsClient;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub struct EdgeTtsLane {
    gen: GeneratorSpec,
    /// R-D: single-attempt budget for dial + exchange (config
    /// `lt_network_deadline_ms`).
    deadline_ms: u32,
    /// The MP3→WAV decode child's exe (the endpoint refuses
    /// uncompressed output; see `mp3dec`).
    mp3_decoder_exe: String,
    /// Monotonic attempt counter: the seed of per-render connection ids.
    attempt: AtomicU64,
}

impl EdgeTtsLane {
    pub fn new(gen: GeneratorSpec, deadline_ms: u32, mp3_decoder_exe: String) -> Self {
        EdgeTtsLane {
            gen,
            deadline_ms,
            mp3_decoder_exe,
            attempt: AtomicU64::new(0),
        }
    }

    /// The protocol half against an OPEN stream — split so tests drive
    /// it without a socket. `seed` names this attempt; it feeds both the
    /// SSML request id and (via `hex_id32`) the connection id.
    fn render_on(
        &self,
        stream: &mut dyn WsStream,
        voice: &VoiceSpec,
        text: &str,
        seed: String,
    ) -> Result<RenderedAudio, AdapterErr> {
        synthesize(
            stream,
            &self.gen,
            &voice.id,
            text,
            &SessionOpts {
                request_seed: seed,
                deadline_ms: u128::from(self.deadline_ms.max(1)),
                cap_ms: self.gen.render_cap_ms,
                prosody: Prosody::default(),
            },
        )
    }
}

impl RenderLane for EdgeTtsLane {
    fn generator(&self) -> &str {
        &self.gen.id
    }

    fn render(
        &self,
        voice: &VoiceSpec,
        text: &str,
        _length_scale: f64,
    ) -> Result<RenderedAudio, AdapterErr> {
        let started = Instant::now();
        let seed = format!(
            "{}-{}",
            self.gen.id,
            self.attempt.fetch_add(1, Ordering::Relaxed)
        );
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        let url = dial_url(&self.gen, &sec_ms_gec(now_ms), &hex_id32(&seed))?;
        let budget = self.deadline_ms.max(1);
        let mut ws = WsClient::connect(&self.gen, &url, budget, budget)
            .map_err(|e| AdapterErr(format!("edge-tts: dial: {e}")))?;
        let audio = self.render_on(&mut ws, voice, text, seed)?;
        // The wire is MP3 (the endpoint's only speech dialect); decode
        // to the organ's WAV before the render leaves the lane — the
        // dispatcher, cache and play child never see compressed audio.
        let mut audio = audio;
        audio.bytes = crate::mp3dec::decode_mp3_to_wav(&self.mp3_decoder_exe, &audio.bytes)?;
        audio.format = crate::adapter::AudioFormat::Wav;
        // Telemetry honesty: the lane's cost is dial + exchange +
        // decode, and the soak counters must see the whole thing.
        audio.elapsed_ms = started.elapsed().as_millis();
        Ok(audio.over_cap_checked())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edgetts::WsFrame;
    use crate::lang::Lang;
    use crate::registry::Lane;

    fn gen() -> GeneratorSpec {
        GeneratorSpec {
            id: "leonas".into(),
            lane: Lane::Network,
            startup_cap_ms: 100,
            render_cap_ms: 1500,
            declared_endpoints: vec!["wss://speech.platform.bing.com".into()],
        }
    }

    fn voice() -> VoiceSpec {
        VoiceSpec {
            id: "lt-LT-LeonasNeural".into(),
            generator: "leonas".into(),
            lang: Lang::Lt,
        }
    }

    /// MP3 wire payload (MPEG-1 Layer III frame sync + payload) — the
    /// endpoint's dialect; passes GA2's MP3 validation.
    fn mp3() -> Vec<u8> {
        let mut v = vec![0xFF, 0xFB, 0x90, 0x00];
        v.extend_from_slice(&[0x33u8; 1_400]);
        v
    }

    fn audio_frame(audio: &[u8]) -> WsFrame {
        let header = b"X-RequestId:ab\r\nPath:audio\r\nContent-Type:audio/mpeg\r\n";
        let mut b = vec![];
        b.extend_from_slice(&(header.len() as u16).to_be_bytes());
        b.extend_from_slice(header);
        b.extend_from_slice(audio);
        WsFrame::Binary(b)
    }

    struct MockWs {
        sent: Vec<String>,
        frames: std::collections::VecDeque<WsFrame>,
        timeouts: Vec<u32>,
    }

    impl WsStream for MockWs {
        fn send_text(&mut self, s: &str) -> Result<(), String> {
            self.sent.push(s.to_string());
            Ok(())
        }
        fn recv_frame(&mut self) -> Result<WsFrame, String> {
            self.frames.pop_front().ok_or_else(|| "closed".into())
        }
        fn set_read_timeout_ms(&mut self, ms: u32) -> Result<(), String> {
            self.timeouts.push(ms);
            Ok(())
        }
    }

    fn scripted(mp3: &[u8]) -> MockWs {
        MockWs {
            sent: vec![],
            frames: [
                WsFrame::Text("X-RequestId:x\r\nPath:turn.start\r\n\r\n{}".into()),
                audio_frame(&mp3[..mp3.len() / 2]),
                audio_frame(&mp3[mp3.len() / 2..]),
                WsFrame::Text("X-RequestId:x\r\nPath:turn.end\r\n\r\n{}".into()),
            ]
            .into_iter()
            .collect(),
            timeouts: vec![],
        }
    }

    #[test]
    fn render_on_happy_path_is_wire_mp3_with_lane_identity() {
        let lane = EdgeTtsLane::new(gen(), 2_500, String::new());
        let mp3 = mp3();
        let mut ws = scripted(&mp3);
        let r = lane
            .render_on(&mut ws, &voice(), "Sveiki.", "leonas-7".into())
            .unwrap();
        // render_on returns the WIRE render — decode to WAV happens in
        // render(), the full lane path.
        assert_eq!(r.format, crate::adapter::AudioFormat::Mp3);
        assert_eq!(r.bytes, mp3);
        assert_eq!(r.generator, "leonas");
        assert_eq!(r.voice, "lt-LT-LeonasNeural");
        assert_eq!(r.cap_ms, 1_500);
        // The R-D budget reached the transport as its read timeout.
        assert_eq!(ws.timeouts, vec![2500]);
        // The voice name rode the SSML; the seed rode the request id.
        assert!(ws.sent[1].contains("lt-LT-LeonasNeural"));
        assert!(ws.sent[1].contains("Sveiki."));
    }

    #[test]
    fn seeds_differ_per_attempt() {
        let lane = EdgeTtsLane::new(gen(), 2_500, String::new());
        let mp3 = mp3();
        let mut a = scripted(&mp3);
        let mut b = scripted(&mp3);
        lane.render_on(&mut a, &voice(), "vienas", "leonas-0".into())
            .unwrap();
        lane.render_on(&mut b, &voice(), "vienas", "leonas-1".into())
            .unwrap();
        assert_ne!(a.sent[1], b.sent[1], "request ids must never repeat");
    }

    #[test]
    fn render_on_maps_transport_loss_to_adapter_err() {
        let lane = EdgeTtsLane::new(gen(), 2_500, String::new());
        let mut ws = MockWs {
            sent: vec![],
            frames: std::collections::VecDeque::new(), // transport dies at once
            timeouts: vec![],
        };
        let e = lane
            .render_on(&mut ws, &voice(), "tekstas", "leonas-0".into())
            .unwrap_err();
        assert!(e.0.contains("transport"), "unexpected err: {e}");
    }
}
