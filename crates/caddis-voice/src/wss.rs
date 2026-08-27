//! wss.rs — the WSS transport (P2 ADAPTER slice d): a schannel TLS client
//! (raw FFI, std-only law, platform::sspi_ffi) + the WebSocket client that
//! implements [`edgetts::WsStream`] EXACTLY — the seam slice (c) left open.
//!
//! GA1 sits AT THE DIAL SITE: [`WsClient::connect`] re-authorizes the URL
//! through [`adapter::authorize_dial`] before a socket exists, so even a
//! caller that skipped `dial_url` cannot make this transport dial anything
//! but the generator's declared endpoint. The TLS context carries the host
//! as its target name — schannel sets SNI AND validates the certificate
//! chain against the system roots with it (automatic validation: we never
//! set MANUAL_CRED_VALIDATION; a bad cert fails the dial, fail-closed).
//!
//! WebSocket layer: RFC 6455 client side. Client frames are MASKED (the
//! RFC requires it), server frames must be unmasked (we close on masked
//! server data — fail-closed), the handshake's `Sec-WebSocket-Accept` is
//! VERIFIED (`base64(SHA-1(key + GUID))`, sha1.rs — a liveness echo, never
//! a security primitive), pings are answered, and a close frame fails the
//! stream: [`edgetts::synthesize`] treats that as the transport error it is.
//!
//! Read timeouts are part of the [`WsStream`] contract: they are enforced
//! at the SOCKET, so the R-D deadline cannot be eaten by a blocked read.
//!
//! Non-Windows: GA1 parsing + refusal work everywhere (tested), the dial
//! itself is Windows-only (schannel) and says so honestly — the vram_probe
//! stub doctrine: a port designs its own transport, it does not inherit a
//! pretend one.

use crate::adapter::authorize_dial;
use crate::edgetts::{WsFrame, WsStream};
use crate::registry::GeneratorSpec;
use crate::sha1::sha1;
use crate::sha256::sha256;

/// RFC 6455 §1.3 — the fixed GUID mixed into the accept key.
pub const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Sanity ceiling for one server frame. Audio chunks arrive ~4-16 KiB;
/// anything near this is a protocol break, not audio.
const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

// ---------------------------------------------------------------------------
// base64 (encode only — the accept check never decodes)
// ---------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// `base64(SHA-1(client_key + GUID))` — the value a correct server echoes.
pub fn accept_key(client_key: &str) -> String {
    let mut v = client_key.as_bytes().to_vec();
    v.extend_from_slice(WS_GUID.as_bytes());
    b64_encode(&sha1(&v))
}

/// `len` unpredictable bytes (std has no RNG): time + counter + pid + a
/// stack address hashed through SHA-256. Enough entropy for WS keys and
/// frame masks, which are protocol liveness, not credentials.
fn nonce(len: usize) -> Vec<u8> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let c = CTR.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let probe = &c as *const u64 as usize;
    let seed = format!(
        "wss:{}:{}:{}:{:x}",
        std::process::id(),
        c,
        probe,
        t.as_nanos()
    );
    let mut out = sha256(seed.as_bytes()).to_vec();
    while out.len() < len {
        let next = sha256(&out);
        out.extend_from_slice(&next);
    }
    out.truncate(len);
    out
}

// ---------------------------------------------------------------------------
// Frame codec (pure — every branch testable without a socket)
// ---------------------------------------------------------------------------

/// A parsed server frame. `Ping`/`Pong` never leave the transport; `Close`
/// fails the stream.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedFrame {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close,
}

/// Build a MASKED client frame (RFC 6455 §5.3): FIN + opcode, mask bit set,
/// length encoding (7-bit / u16 / u64), 4-byte mask, XOR payload.
pub fn encode_client_frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut f = Vec::with_capacity(10 + 4 + payload.len());
    f.push(0x80 | opcode);
    let m = 0x80u8;
    if payload.len() < 126 {
        f.push(m | payload.len() as u8);
    } else if payload.len() <= 0xFFFF {
        f.push(m | 126);
        f.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        f.push(m | 127);
        f.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    f.extend_from_slice(&mask);
    for (i, b) in payload.iter().enumerate() {
        f.push(b ^ mask[i % 4]);
    }
    f
}

/// Parse one UNMASKED server frame from the head of `buf`. `Ok(None)` = not
/// enough bytes yet. Everything unexpected is an error — this lane fails
/// closed on protocol surprises.
pub fn parse_server_frame(buf: &[u8]) -> Result<Option<(ParsedFrame, usize)>, String> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let b0 = buf[0];
    let b1 = buf[1];
    let fin = b0 & 0x80 != 0;
    let opcode = b0 & 0x0F;
    if b1 & 0x80 != 0 {
        return Err("websocket: server sent a MASKED frame — protocol violation".into());
    }
    let control = matches!(opcode, 0x8..=0xA);
    let len7 = (b1 & 0x7F) as usize;
    if control && len7 > 125 {
        return Err("websocket: control frame longer than 125 bytes".into());
    }
    let (idx, len) = match len7 {
        126 => {
            if buf.len() < 4 {
                return Ok(None);
            }
            (4usize, u16::from_be_bytes([buf[2], buf[3]]) as usize)
        }
        127 => {
            if buf.len() < 10 {
                return Ok(None);
            }
            let n = u64::from_be_bytes([
                buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8], buf[9],
            ]);
            if n > MAX_FRAME_BYTES as u64 {
                return Err("websocket: frame length beyond the sanity ceiling".into());
            }
            (10usize, n as usize)
        }
        n => (2usize, n),
    };
    let end = idx.checked_add(len).ok_or("websocket: length overflow")?;
    if end > MAX_FRAME_BYTES {
        return Err("websocket: frame beyond the sanity ceiling".into());
    }
    if buf.len() < end {
        return Ok(None);
    }
    let payload = &buf[idx..end];
    if !fin {
        return Err("websocket: fragmentation is not part of this lane's protocol".into());
    }
    let frame = match opcode {
        0x0 => return Err("websocket: continuation frame without a start".into()),
        0x1 => ParsedFrame::Text(
            String::from_utf8(payload.to_vec())
                .map_err(|_| "websocket: server text frame is not valid UTF-8".to_string())?,
        ),
        0x2 => ParsedFrame::Binary(payload.to_vec()),
        0x8 => ParsedFrame::Close,
        0x9 => ParsedFrame::Ping(payload.to_vec()),
        0xA => ParsedFrame::Pong(payload.to_vec()),
        other => return Err(format!("websocket: unknown opcode {other:#x}")),
    };
    Ok(Some((frame, end)))
}

/// Verify the handshake response head (everything before `\r\n\r\n`):
/// status 101 + a matching `Sec-WebSocket-Accept`.
pub fn check_handshake_response(head: &str, expected_accept: &str) -> Result<(), String> {
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or("");
    if !(status.contains(" 101 ") || status.ends_with(" 101")) {
        return Err(format!("wss: upgrade refused — {status}"));
    }
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("sec-websocket-accept") {
                if value.trim() == expected_accept {
                    return Ok(());
                }
                return Err("wss: Sec-WebSocket-Accept mismatch — wrong or hostile server".into());
            }
        }
    }
    Err("wss: response is missing Sec-WebSocket-Accept".into())
}

/// The request-target portion of an absolute `wss://` URL (path + query,
/// `#fragment` dropped). `/` when the authority is all there is.
pub fn url_path(url: &str) -> String {
    let Some((_, rest)) = url.split_once("://") else {
        return "/".into();
    };
    let start = match rest.find(['/', '?', '#']) {
        Some(i) => i,
        None => return "/".into(),
    };
    let s = rest[start..].split('#').next().unwrap_or("");
    if s.is_empty() {
        "/".into()
    } else if s.starts_with('?') {
        format!("/{s}")
    } else {
        s.into()
    }
}

// ---------------------------------------------------------------------------
// TLS over schannel (Windows)
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod tls {
    use crate::platform::*;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
    use std::os::raw::c_void;
    use std::time::Duration;

    const ISC_REQ_CLIENT: u32 = ISC_REQ_REPLAY_DETECT
        | ISC_REQ_SEQUENCE_DETECT
        | ISC_REQ_CONFIDENTIALITY
        | ISC_REQ_ALLOCATE_MEMORY
        | ISC_REQ_STREAM
        | ISC_REQ_INTEGRITY;

    fn status_hex(st: SspiStatus) -> String {
        format!("{:#010x}", st as u32)
    }

    /// One schannel TLS client stream over a connected TcpStream.
    pub struct TlsStream {
        sock: TcpStream,
        cred: SecHandle,
        ctx: SecHandle,
        sizes: StreamSizes,
        /// Ciphertext read from the socket, not yet consumed by DecryptMessage.
        cipher: Vec<u8>,
        /// Decrypted app data produced early (before the caller asked to
        /// read) — handed out by the next read_plain.
        pending_plain: Vec<u8>,
        /// The host as UTF-16 — kept for the shutdown-time ISC target name.
        host_w: Vec<u16>,
    }

    /// One DecryptMessage attempt over the whole cipher buffer.
    #[derive(Debug)]
    enum StepOutcome {
        /// SEC_E_OK: app data out; `extra_off` = unconsumed tail offset
        /// (None = the whole input was consumed).
        Data {
            out: Vec<u8>,
            extra_off: Option<usize>,
        },
        /// SEC_I_RENEGOTIATE: fed bytes consumed, `extra_off` = tail.
        Renegotiate { extra_off: Option<usize> },
        /// SEC_E_INCOMPLETE_MESSAGE: cipher MUST be kept as-is.
        Incomplete,
    }

    impl TlsStream {
        /// Connect + full TLS handshake. The read timeout applies from the
        /// first handshake read on.
        pub fn connect(
            host: &str,
            port: u16,
            connect_timeout: Duration,
            read_timeout: Duration,
        ) -> Result<Self, String> {
            let addr = (host, port)
                .to_socket_addrs()
                .map_err(|e| format!("tls: cannot resolve {host}: {e}"))?
                .find(SocketAddr::is_ipv4)
                .ok_or_else(|| format!("tls: no address for {host}"))?;
            let sock = TcpStream::connect_timeout(&addr, connect_timeout)
                .map_err(|e| format!("tls: connect {host}:{port} failed: {e}"))?;
            sock.set_read_timeout(Some(read_timeout))
                .map_err(|e| format!("tls: set_read_timeout: {e}"))?;
            sock.set_nodelay(true).ok();

            let host_w: Vec<u16> = host.encode_utf16().chain(std::iter::once(0)).collect();

            // Credentials: SCH_CREDENTIALS v5, no client cert, default
            // system TLS parameters. Automatic chain validation stays ON.
            let cred_data = SchCredentials {
                dw_version: SCH_CREDENTIALS_VERSION,
                dw_cred_format: 0,
                c_creds: 0,
                pa_cred: std::ptr::null(),
                h_root_store: std::ptr::null_mut(),
                c_mappers: 0,
                aph_mappers: std::ptr::null(),
                dw_session_lifespan: 0,
                dw_flags: SCH_CRED_NO_DEFAULT_CREDS,
                c_tls_parameters: 0,
                p_tls_parameters: std::ptr::null(),
            };
            let unisp: Vec<u16> = "Microsoft Unified Security Protocol Provider"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut cred = SecHandle::null();
            let st = unsafe {
                AcquireCredentialsHandleW(
                    std::ptr::null(),
                    unisp.as_ptr(),
                    SECPKG_CRED_OUTBOUND,
                    std::ptr::null(),
                    &cred_data as *const SchCredentials as *const c_void,
                    std::ptr::null(),
                    std::ptr::null(),
                    &mut cred,
                    std::ptr::null_mut(),
                )
            };
            if st != SEC_E_OK {
                return Err(format!("tls: AcquireCredentialsHandle {}", status_hex(st)));
            }

            let mut tls = TlsStream {
                sock,
                cred,
                ctx: SecHandle::null(),
                sizes: StreamSizes {
                    cb_header: 0,
                    cb_trailer: 0,
                    cb_maximum_message: 16 * 1024,
                    c_buffers: 0,
                    cb_block_size: 0,
                },
                cipher: Vec::new(),
                pending_plain: Vec::new(),
                host_w,
            };
            tls.handshake()?;
            // TLS 1.3: the server's post-handshake flight (NewSessionTicket)
            // can be pending when ISC returns OK; schannel finalizes the
            // client→server app keys only after consuming it — a first
            // EncryptMessage before that fails SEC_E_ENCRYPT_FAILURE. Bounded
            // drain: ONE short read into the cipher buffer, every error
            // ignored (nothing pending is the common case, not a defect).
            {
                let saved = read_timeout;
                let _ = tls.sock.set_read_timeout(Some(Duration::from_millis(250)));
                let mut tmp = [0u8; 16 * 1024];
                match tls.sock.read(&mut tmp) {
                    Ok(n) if n > 0 => tls.cipher.extend_from_slice(&tmp[..n]),
                    _ => {}
                }
                let _ = tls.sock.set_read_timeout(Some(saved));
            }
            Ok(tls)
        }

        fn handshake(&mut self) -> Result<(), String> {
            self.isc_loop(Vec::new(), false)
        }

        /// The shared InitializeSecurityContext exchange — the initial
        /// handshake AND renegotiation both run here. `resumed`: the
        /// context already exists (renegotiation — bing's edge asks for
        /// one right after the TLS 1.3 handshake; an EncryptMessage in
        /// that window is exactly SEC_E_ENCRYPT_FAILURE). Input desc is
        /// [TOKEN, EMPTY] — schannel reports EXTRA/MISSING in slot 2
        /// (schannel-crate-proven shape).
        fn isc_loop(&mut self, mut pending: Vec<u8>, resumed: bool) -> Result<(), String> {
            let mut first = !resumed;
            let mut attrs: u32 = 0;
            // ALPN "http/1.1" — the edge's front door enforces it (a
            // bare schannel hello without ALPN gets the whole session
            // 403'd). SEC_APPLICATION_PROTOCOLS byte shape, mirrored from
            // the proven reference:
            // [u32 lists_size][u32 ext=ALPN(2)][u16 list_size][wire],
            // wire = [len][bytes] per protocol. 4-aligned for the struct.
            #[repr(C, align(4))]
            struct AlpnBlob([u8; 20]);
            let wire: &[u8] = b"\x08http/1.1";
            let lists_size = (4 + 2 + wire.len()) as u32;
            let mut alpn = AlpnBlob([0u8; 20]);
            alpn.0[0..4].copy_from_slice(&lists_size.to_le_bytes());
            alpn.0[4..8]
                .copy_from_slice(&SEC_APPLICATION_PROTOCOL_NEGOTIATION_EXT_ALPN.to_le_bytes());
            alpn.0[8..10].copy_from_slice(&(wire.len() as u16).to_le_bytes());
            alpn.0[10..10 + wire.len()].copy_from_slice(wire);
            let alpn_len = 10 + wire.len();
            loop {
                // Input: whatever server bytes we hold (first call: none),
                // plus the ALPN list (the reference sends it on every
                // handshake ISC call).
                let mut in_bufs = [
                    SecBuffer {
                        cb_buffer: pending.len() as u32,
                        buffer_type: SECBUFFER_TOKEN,
                        pv_buffer: if pending.is_empty() {
                            std::ptr::null_mut()
                        } else {
                            pending.as_mut_ptr() as *mut c_void
                        },
                    },
                    SecBuffer {
                        cb_buffer: 0,
                        buffer_type: 0,
                        pv_buffer: std::ptr::null_mut(),
                    },
                    SecBuffer {
                        cb_buffer: alpn_len as u32,
                        buffer_type: SECBUFFER_APPLICATION_PROTOCOLS,
                        pv_buffer: alpn.0.as_ptr() as *mut c_void,
                    },
                ];
                let in_desc = SecBufferDesc {
                    ul_version: SECBUFFER_VERSION,
                    c_buffers: 3,
                    p_buffers: in_bufs.as_mut_ptr(),
                };
                let mut out_buf = SecBuffer {
                    cb_buffer: 0,
                    buffer_type: SECBUFFER_TOKEN,
                    pv_buffer: std::ptr::null_mut(),
                };
                let mut out_desc = SecBufferDesc {
                    ul_version: SECBUFFER_VERSION,
                    c_buffers: 1,
                    p_buffers: &mut out_buf,
                };
                let ctx_ptr: *const SecHandle = if first { std::ptr::null() } else { &self.ctx };
                let st = unsafe {
                    InitializeSecurityContextW(
                        &self.cred,
                        ctx_ptr,
                        self.host_w.as_ptr(),
                        ISC_REQ_CLIENT,
                        0,
                        SECURITY_NATIVE_DREP,
                        if in_desc.c_buffers == 0 {
                            std::ptr::null()
                        } else {
                            &in_desc
                        },
                        0,
                        &mut self.ctx,
                        &mut out_desc,
                        &mut attrs,
                        std::ptr::null_mut(),
                    )
                };
                // Send whatever flight schannel produced, then free it.
                if out_buf.cb_buffer > 0 && !out_buf.pv_buffer.is_null() {
                    let token = unsafe {
                        std::slice::from_raw_parts(
                            out_buf.pv_buffer as *const u8,
                            out_buf.cb_buffer as usize,
                        )
                    };
                    let send = self.sock.write_all(token);
                    unsafe { FreeContextBuffer(out_buf.pv_buffer) };
                    send.map_err(|e| format!("tls: send flight: {e}"))?;
                }
                match st {
                    SEC_E_OK => {
                        // Leftover server bytes (early app data / EXTRA) become
                        // the first ciphertext for DecryptMessage.
                        // No EXTRA reported = the WHOLE input was consumed;
                        // keeping it would feed DecryptMessage already-eaten
                        // handshake bytes (INVALID_TOKEN downstream).
                        match extra_offset(&in_desc, &in_bufs[0], &pending) {
                            Some(off) => {
                                pending.drain(..off);
                            }
                            None => pending.clear(),
                        }
                        self.cipher = pending;
                        let mut sizes = StreamSizes {
                            cb_header: 0,
                            cb_trailer: 0,
                            cb_maximum_message: 0,
                            c_buffers: 0,
                            cb_block_size: 0,
                        };
                        let st = unsafe {
                            QueryContextAttributesW(
                                &self.ctx,
                                SECPKG_ATTR_STREAM_SIZES,
                                &mut sizes as *mut StreamSizes as *mut c_void,
                            )
                        };
                        if st != SEC_E_OK || sizes.cb_maximum_message == 0 {
                            return Err(format!("tls: stream sizes {}", status_hex(st)));
                        }
                        self.sizes = sizes;
                        return Ok(());
                    }
                    SEC_I_CONTINUE_NEEDED => {
                        match extra_offset(&in_desc, &in_bufs[0], &pending) {
                            Some(off) => {
                                pending.drain(..off);
                            }
                            None => pending.clear(),
                        }
                        first = false;
                        // Pull the next server flight (bounded by the socket
                        // read timeout — a stalled peer fails, never hangs).
                        let mut tmp = [0u8; 16 * 1024];
                        let n = self
                            .sock
                            .read(&mut tmp)
                            .map_err(|e| format!("tls: handshake read: {e}"))?;
                        if n == 0 {
                            return Err("tls: server closed mid-handshake".into());
                        }
                        pending.extend_from_slice(&tmp[..n]);
                    }
                    other => {
                        return Err(format!("tls: handshake status {}", status_hex(other)));
                    }
                }
            }
        }

        pub fn set_read_timeout_ms(&mut self, ms: u32) -> Result<(), String> {
            self.sock
                .set_read_timeout(Some(Duration::from_millis(u64::from(ms.max(1)))))
                .map_err(|e| format!("tls: set_read_timeout: {e}"))
        }

        /// Encrypt + send. Data is chunked by schannel's max message size.
        pub fn write_plain(&mut self, data: &[u8]) -> Result<(), String> {
            let chunk_len = (self.sizes.cb_maximum_message as usize).max(1);
            // A renegotiation request parked in `cipher` makes the next
            // EncryptMessage fail (SEC_E_ENCRYPT_FAILURE) — process it
            // first: early app data lands in pending_plain for the read
            // side, a renegotiation completes before we encrypt.
            while !self.cipher.is_empty() {
                let before = self.cipher.len();
                match self.decrypt_step()? {
                    StepOutcome::Renegotiate { extra_off } => {
                        match extra_off {
                            Some(off) => {
                                self.cipher.drain(..off);
                            }
                            None => self.cipher.clear(),
                        }
                        let tail = std::mem::take(&mut self.cipher);
                        self.isc_loop(tail, true)?;
                    }
                    StepOutcome::Data { out, extra_off } => {
                        match extra_off {
                            Some(off) => {
                                self.cipher.drain(..off);
                            }
                            None => self.cipher.clear(),
                        }
                        if !out.is_empty() {
                            self.pending_plain.extend_from_slice(&out);
                        }
                        if self.cipher.len() >= before {
                            break; // no progress: incomplete record head
                        }
                    }
                    StepOutcome::Incomplete => break,
                }
            }
            for chunk in data.chunks(chunk_len) {
                let hdr = self.sizes.cb_header as usize;
                let trl = self.sizes.cb_trailer as usize;
                let mut record = vec![0u8; hdr + chunk.len() + trl];
                let data_off = hdr;
                record[data_off..data_off + chunk.len()].copy_from_slice(chunk);
                let mut bufs = [
                    SecBuffer {
                        cb_buffer: hdr as u32,
                        buffer_type: SECBUFFER_STREAM_HEADER,
                        pv_buffer: record.as_mut_ptr() as *mut c_void,
                    },
                    SecBuffer {
                        cb_buffer: chunk.len() as u32,
                        buffer_type: SECBUFFER_DATA,
                        pv_buffer: unsafe { record.as_mut_ptr().add(data_off) as *mut c_void },
                    },
                    SecBuffer {
                        cb_buffer: trl as u32,
                        buffer_type: SECBUFFER_STREAM_TRAILER,
                        pv_buffer: unsafe {
                            record.as_mut_ptr().add(data_off + chunk.len()) as *mut c_void
                        },
                    },
                    // MSDN EncryptMessage example shape: schannel wants a
                    // fourth, EMPTY slot after the trailer.
                    SecBuffer {
                        cb_buffer: 0,
                        buffer_type: 0,
                        pv_buffer: std::ptr::null_mut(),
                    },
                ];
                let mut desc = SecBufferDesc {
                    ul_version: SECBUFFER_VERSION,
                    c_buffers: 4,
                    p_buffers: bufs.as_mut_ptr(),
                };
                let st = unsafe { EncryptMessage(&self.ctx, 0, &mut desc, 0) };
                if st != SEC_E_OK {
                    return Err(format!("tls: encrypt {}", status_hex(st)));
                }
                // Send each buffer by its POST-call length (schannel may use
                // less than reserved).
                let mut out = Vec::with_capacity(record.len());
                for b in &bufs[..desc.c_buffers as usize] {
                    if b.cb_buffer > 0 && !b.pv_buffer.is_null() {
                        out.extend_from_slice(unsafe {
                            std::slice::from_raw_parts(
                                b.pv_buffer as *const u8,
                                b.cb_buffer as usize,
                            )
                        });
                    }
                }
                self.sock
                    .write_all(&out)
                    .map_err(|e| format!("tls: send: {e}"))?;
            }
            Ok(())
        }

        /// Decrypt until at least one plaintext byte exists. `Err` on
        /// timeout, close-notify, or protocol failure — never a silent
        /// empty return.
        pub fn read_plain(&mut self) -> Result<Vec<u8>, String> {
            loop {
                if !self.pending_plain.is_empty() {
                    return Ok(std::mem::take(&mut self.pending_plain));
                }
                if !self.cipher.is_empty() {
                    let before = self.cipher.len();
                    match self.decrypt_step()? {
                        StepOutcome::Renegotiate { extra_off } => {
                            // Bing's edge renegotiates right after the TLS
                            // handshake. The fed ciphertext was consumed by
                            // the decrypt call — drain it, then re-run the
                            // ISC exchange over the existing context;
                            // leftover ciphertext stays queued in `cipher`.
                            match extra_off {
                                Some(off) => {
                                    self.cipher.drain(..off);
                                }
                                None => self.cipher.clear(),
                            }
                            let tail = std::mem::take(&mut self.cipher);
                            self.isc_loop(tail, true)?;
                            continue;
                        }
                        StepOutcome::Data { out, extra_off } => {
                            match extra_off {
                                Some(off) => {
                                    self.cipher.drain(..off);
                                }
                                None => self.cipher.clear(),
                            }
                            if !out.is_empty() {
                                return Ok(out);
                            }
                            if self.cipher.len() < before {
                                // Records consumed, no app data (control
                                // messages): decrypt what remains.
                                continue;
                            }
                            // No progress: the buffer needs more bytes.
                        }
                        StepOutcome::Incomplete => { /* keep cipher, read more */ }
                    }
                }
                let mut tmp = [0u8; 16 * 1024];
                let n = self
                    .sock
                    .read(&mut tmp)
                    .map_err(|e| format!("tls: read: {e}"))?;
                if n == 0 {
                    return Err("tls: connection closed by server".into());
                }
                self.cipher.extend_from_slice(&tmp[..n]);
            }
        }

        fn decrypt_step(&mut self) -> Result<StepOutcome, String> {
            let base = self.cipher.as_mut_ptr() as *mut c_void;
            let mut bufs = [
                SecBuffer {
                    cb_buffer: self.cipher.len() as u32,
                    buffer_type: SECBUFFER_DATA,
                    pv_buffer: base,
                },
                SecBuffer {
                    cb_buffer: 0,
                    buffer_type: 0,
                    pv_buffer: std::ptr::null_mut(),
                },
                SecBuffer {
                    cb_buffer: 0,
                    buffer_type: 0,
                    pv_buffer: std::ptr::null_mut(),
                },
                SecBuffer {
                    cb_buffer: 0,
                    buffer_type: 0,
                    pv_buffer: std::ptr::null_mut(),
                },
            ];
            let mut desc = SecBufferDesc {
                ul_version: SECBUFFER_VERSION,
                c_buffers: 4,
                p_buffers: bufs.as_mut_ptr(),
            };
            let mut qop: u32 = 0;
            let st = unsafe { DecryptMessage(&self.ctx, &mut desc, 0, &mut qop) };
            // EXTRA applies to OK and RENEGOTIATE alike: both consumed
            // some ciphertext and may leave a tail.
            let mut extra_off: Option<usize> = None;
            for b in &bufs[..desc.c_buffers as usize] {
                if b.buffer_type == SECBUFFER_EXTRA && !b.pv_buffer.is_null() {
                    let off = b.pv_buffer as usize - base as usize;
                    if off <= self.cipher.len() {
                        extra_off = Some(off);
                    }
                }
            }
            match st {
                SEC_E_OK => {
                    let mut out = Vec::new();
                    for b in &bufs[..desc.c_buffers as usize] {
                        if b.buffer_type == SECBUFFER_DATA
                            && b.cb_buffer > 0
                            && !b.pv_buffer.is_null()
                        {
                            out.extend_from_slice(unsafe {
                                std::slice::from_raw_parts(
                                    b.pv_buffer as *const u8,
                                    b.cb_buffer as usize,
                                )
                            });
                        }
                    }
                    Ok(StepOutcome::Data { out, extra_off })
                }
                // The fed ciphertext was CONSUMED (the trigger message was
                // processed internally): drain it, then re-run ISC over the
                // existing context with the REMAINING bytes (usually none)
                // — the renegotiation ClientHello needs no server input.
                SEC_I_RENEGOTIATE => Ok(StepOutcome::Renegotiate { extra_off }),
                SEC_E_INCOMPLETE_MESSAGE => Ok(StepOutcome::Incomplete),
                SEC_I_CONTEXT_EXPIRED => Err("tls: server sent close notify".into()),
                other => Err(format!("tls: decrypt {}", status_hex(other))),
            }
        }

        /// Best-effort close_notify: ApplyControlToken(SCHANNEL_SHUTDOWN) +
        /// one more ISC produces the shutdown record; send it and tear down.
        fn send_close_notify(&mut self) {
            if self.ctx.is_null() {
                return;
            }
            let mut token: u32 = SCHANNEL_SHUTDOWN;
            let mut tok_buf = SecBuffer {
                cb_buffer: 4,
                buffer_type: SECBUFFER_TOKEN,
                pv_buffer: &mut token as *mut u32 as *mut c_void,
            };
            let tok_desc = SecBufferDesc {
                ul_version: SECBUFFER_VERSION,
                c_buffers: 1,
                p_buffers: &mut tok_buf,
            };
            if unsafe { ApplyControlToken(&self.ctx, &tok_desc) } == SEC_E_OK {
                let mut out_buf = SecBuffer {
                    cb_buffer: 0,
                    buffer_type: SECBUFFER_TOKEN,
                    pv_buffer: std::ptr::null_mut(),
                };
                let mut out_desc = SecBufferDesc {
                    ul_version: SECBUFFER_VERSION,
                    c_buffers: 1,
                    p_buffers: &mut out_buf,
                };
                let mut attrs: u32 = 0;
                let st = unsafe {
                    InitializeSecurityContextW(
                        &self.cred,
                        &self.ctx,
                        self.host_w.as_ptr(),
                        ISC_REQ_CLIENT,
                        0,
                        SECURITY_NATIVE_DREP,
                        std::ptr::null(),
                        0,
                        &mut self.ctx,
                        &mut out_desc,
                        &mut attrs,
                        std::ptr::null_mut(),
                    )
                };
                if out_buf.cb_buffer > 0 && !out_buf.pv_buffer.is_null() {
                    let record = unsafe {
                        std::slice::from_raw_parts(
                            out_buf.pv_buffer as *const u8,
                            out_buf.cb_buffer as usize,
                        )
                    };
                    let _ = self.sock.write_all(record);
                    unsafe { FreeContextBuffer(out_buf.pv_buffer) };
                }
                let _ = st;
            }
            unsafe {
                DeleteSecurityContext(&self.ctx);
                FreeCredentialsHandle(&mut self.cred);
            }
        }
    }

    impl Drop for TlsStream {
        fn drop(&mut self) {
            self.send_close_notify();
        }
    }

    /// If schannel rewrote the input TOKEN buffer to EXTRA, the offset of
    /// the unconsumed tail within `pending`.
    fn extra_offset(desc: &SecBufferDesc, first: &SecBuffer, pending: &[u8]) -> Option<usize> {
        if desc.c_buffers == 0 || pending.is_empty() || first.pv_buffer.is_null() {
            return None;
        }
        let base = pending.as_ptr() as usize;
        for b in unsafe { std::slice::from_raw_parts(desc.p_buffers, desc.c_buffers as usize) } {
            if b.buffer_type == SECBUFFER_EXTRA && !b.pv_buffer.is_null() {
                let off = b.pv_buffer as usize - base;
                if off <= pending.len() {
                    return Some(off);
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// WsClient — the WsStream implementation
// ---------------------------------------------------------------------------

pub struct WsClient {
    #[cfg(windows)]
    tls: tls::TlsStream,
    /// Decrypted plaintext not yet parsed into a complete frame.
    frame_buf: Vec<u8>,
    read_timeout_ms: u32,
}

impl std::fmt::Debug for WsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WsClient")
            .field("read_timeout_ms", &self.read_timeout_ms)
            .field("buffered_plaintext", &self.frame_buf.len())
            .finish_non_exhaustive()
    }
}

impl WsClient {
    /// GA1-gated dial + TLS + WebSocket upgrade. The URL is re-authorized
    /// HERE — the transport itself refuses to dial anything a generator did
    /// not declare, even if the caller skipped `dial_url`.
    pub fn connect(
        gen: &GeneratorSpec,
        url: &str,
        connect_timeout_ms: u32,
        read_timeout_ms: u32,
    ) -> Result<Self, String> {
        let plan = authorize_dial(gen, url).map_err(|e| e.0)?;
        if plan.scheme != "wss" {
            return Err(format!(
                "wss: scheme {} is not a websocket dial",
                plan.scheme
            ));
        }
        let path = url_path(url);

        #[cfg(windows)]
        {
            let mut tls = tls::TlsStream::connect(
                &plan.host,
                plan.port,
                std::time::Duration::from_millis(u64::from(connect_timeout_ms.max(1))),
                std::time::Duration::from_millis(u64::from(read_timeout_ms.max(1))),
            )?;
            let key = b64_encode(&nonce(16));
            // Header set mirrors the reference client exactly (stale
            // Sec-MS-GEC-Version or a missing muid cookie = plain 403).
            let muid: String = nonce(16).iter().map(|b| format!("{b:02X}")).collect();
            let req = format!(
                "GET {path} HTTP/1.1\r\n\
                 Host: {host}\r\n\
                 Upgrade: websocket\r\n\
                 Connection: Upgrade\r\n\
                 Sec-WebSocket-Key: {key}\r\n\
                 Sec-WebSocket-Version: 13\r\n\
                 Origin: chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold\r\n\
                 Cookie: muid={muid};\r\n\
                 Pragma: no-cache\r\n\
                 Cache-Control: no-cache\r\n\
                 Accept-Encoding: gzip, deflate, br, zstd\r\n\
                 Accept-Language: en-US,en;q=0.9\r\n\
                 User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
                 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 \
                 Safari/537.36 Edg/143.0.0.0\r\n\
                 \r\n",
                host = plan.host,
            );
            tls.write_plain(req.as_bytes())?;
            let expected = accept_key(&key);
            let mut head: Vec<u8> = Vec::new();
            let frame_start;
            loop {
                let chunk = tls.read_plain()?;
                head.extend_from_slice(&chunk);
                if let Some(pos) = find_head_end(&head) {
                    let head_str = String::from_utf8_lossy(&head[..pos]).into_owned();
                    check_handshake_response(&head_str, &expected)?;
                    frame_start = pos + 4;
                    break;
                }
                if head.len() > 32 * 1024 {
                    return Err("wss: handshake response head exceeds 32 KiB".into());
                }
            }
            Ok(WsClient {
                tls,
                frame_buf: head[frame_start..].to_vec(),
                read_timeout_ms,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Err("wss transport is Windows-only (schannel)".into())
        }
    }

    #[cfg(windows)]
    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), String> {
        let mask: [u8; 4] = nonce(4).try_into().expect("nonce(4) is 4 bytes");
        let frame = encode_client_frame(opcode, payload, mask);
        self.tls.write_plain(&frame)
    }
}

/// Offset of the `\r\n\r\n` that ends a response head, if present.
fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

#[cfg(windows)]
impl WsStream for WsClient {
    fn send_text(&mut self, s: &str) -> Result<(), String> {
        self.send_frame(0x1, s.as_bytes())
    }

    fn recv_frame(&mut self) -> Result<WsFrame, String> {
        loop {
            if let Some((frame, used)) = parse_server_frame(&self.frame_buf)? {
                self.frame_buf.drain(..used);
                return match frame {
                    ParsedFrame::Text(s) => Ok(WsFrame::Text(s)),
                    ParsedFrame::Binary(b) => Ok(WsFrame::Binary(b)),
                    ParsedFrame::Ping(p) => {
                        self.send_frame(0xA, &p)?;
                        continue;
                    }
                    ParsedFrame::Pong(_) => continue,
                    ParsedFrame::Close => Err("websocket: server closed the stream".into()),
                };
            }
            let chunk = self.tls.read_plain()?;
            self.frame_buf.extend_from_slice(&chunk);
        }
    }

    fn set_read_timeout_ms(&mut self, ms: u32) -> Result<(), String> {
        self.read_timeout_ms = ms;
        self.tls.set_read_timeout_ms(ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- accept / base64 ---

    #[test]
    fn b64_vectors() {
        assert_eq!(b64_encode(b""), "");
        assert_eq!(b64_encode(b"f"), "Zg==");
        assert_eq!(b64_encode(b"fo"), "Zm8=");
        assert_eq!(b64_encode(b"foo"), "Zm9v");
        assert_eq!(b64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(b64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn rfc6455_accept_example() {
        // RFC 6455 §1.3: this exact key/guid pair.
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    // --- frame codec ---

    #[test]
    fn client_text_frame_shape() {
        // Known mask → byte-exact frame: FIN|text, mask bit + len 5, mask,
        // XOR payload.
        let f = encode_client_frame(0x1, b"Hello", [0x37, 0xfa, 0x21, 0x3d]);
        assert_eq!(
            f,
            vec![
                0x81,
                0x85,
                0x37,
                0xfa,
                0x21,
                0x3d,
                b'H' ^ 0x37,
                b'e' ^ 0xfa,
                b'l' ^ 0x21,
                b'l' ^ 0x3d,
                b'o' ^ 0x37
            ]
        );
    }

    #[test]
    fn client_frame_extended_lengths() {
        let mask = [1, 2, 3, 4];
        let payload = vec![7u8; 300];
        let f = encode_client_frame(0x2, &payload, mask);
        assert_eq!(&f[..4], &[0x82, 0x80 | 126, 0x01, 0x2c]);
        assert_eq!(f.len(), 4 + 4 + 300);
        let big = vec![9u8; 70_000];
        let f = encode_client_frame(0x2, &big, mask);
        assert_eq!(f[1], 0x80 | 127);
        assert_eq!(f.len(), 10 + 4 + 70_000);
    }

    #[test]
    fn parse_server_frames() {
        // Unmasked text "abc".
        let (f, n) = parse_server_frame(&[0x81, 0x03, b'a', b'b', b'c'])
            .unwrap()
            .expect("complete");
        assert_eq!(f, ParsedFrame::Text("abc".into()));
        assert_eq!(n, 5);
        // Binary with u16 length.
        let mut b = vec![0x82, 126, 0x00, 0x04];
        b.extend_from_slice(&[1, 2, 3, 4]);
        let (f, n) = parse_server_frame(&b).unwrap().expect("complete");
        assert_eq!(f, ParsedFrame::Binary(vec![1, 2, 3, 4]));
        assert_eq!(n, 8);
        // Ping and close.
        let (f, _) = parse_server_frame(&[0x89, 0x02, 0x00, 0x00])
            .unwrap()
            .unwrap();
        assert_eq!(f, ParsedFrame::Ping(vec![0, 0]));
        let (f, _) = parse_server_frame(&[0x88, 0x00]).unwrap().unwrap();
        assert_eq!(f, ParsedFrame::Close);
    }

    #[test]
    fn parse_incomplete_is_none_not_error() {
        assert_eq!(parse_server_frame(&[]).unwrap(), None);
        assert_eq!(parse_server_frame(&[0x81]).unwrap(), None);
        assert_eq!(parse_server_frame(&[0x82, 126, 0x01]).unwrap(), None);
        // Header claims 4 bytes, only 3 present.
        assert_eq!(parse_server_frame(&[0x82, 0x04, 1, 2, 3]).unwrap(), None);
    }

    #[test]
    fn parse_fail_closed() {
        // Masked server frame.
        assert!(parse_server_frame(&[0x81, 0x85, 0, 0, 0, 0, 0]).is_err());
        // Fragmented.
        assert!(parse_server_frame(&[0x01, 0x01, b'x']).is_err());
        // Bare continuation.
        assert!(parse_server_frame(&[0x00, 0x00]).is_err());
        // Unknown opcode.
        assert!(parse_server_frame(&[0x83, 0x00]).is_err());
        // Control frame > 125.
        assert!(parse_server_frame(&[0x89, 0x7E, 0x00, 0x80]).is_err());
        // Oversized u64 length.
        let mut big = vec![0x82, 0x80 | 127];
        big.extend_from_slice(&u64::MAX.to_be_bytes());
        assert!(parse_server_frame(&big).is_err());
    }

    // --- handshake response ---

    #[test]
    fn handshake_response_check() {
        let accept = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";
        let ok = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n"
        );
        assert_eq!(check_handshake_response(&ok, accept), Ok(()));
        let lower =
            format!("HTTP/1.1 101 Switching Protocols\r\nsec-websocket-accept: {accept}\r\n");
        assert_eq!(check_handshake_response(&lower, accept), Ok(()));
        assert!(check_handshake_response(&ok, "wrong").is_err());
        let refused = "HTTP/1.1 403 Forbidden\r\n\r\n";
        assert!(check_handshake_response(refused, accept).is_err());
        let no_accept = "HTTP/1.1 101 Switching Protocols\r\n\r\n";
        assert!(check_handshake_response(no_accept, accept).is_err());
    }

    // --- URL path ---

    #[test]
    fn url_path_variants() {
        assert_eq!(url_path("wss://h:443"), "/");
        assert_eq!(url_path("wss://h:443/"), "/");
        assert_eq!(
            url_path("wss://h:443/consumer/speech?x=1&y=2"),
            "/consumer/speech?x=1&y=2"
        );
        assert_eq!(url_path("wss://h:443?Trusted=1"), "/?Trusted=1");
        assert_eq!(url_path("wss://h:443/a?b=1#frag"), "/a?b=1");
    }

    // --- GA1 at the dial site (all platforms, no socket involved) ---

    #[test]
    fn connect_refuses_undeclared_and_offline() {
        use crate::registry::Lane;
        let offline = GeneratorSpec {
            id: "piper".into(),
            lane: Lane::Offline,
            startup_cap_ms: 100,
            render_cap_ms: 1500,
            declared_endpoints: vec![],
        };
        let e = WsClient::connect(&offline, "wss://speech.platform.bing.com:443/x", 1000, 1000)
            .unwrap_err();
        assert!(e.contains("GA1"), "{e}");

        let wrong_host = GeneratorSpec {
            id: "leonas".into(),
            lane: Lane::Network,
            startup_cap_ms: 100,
            render_cap_ms: 1500,
            declared_endpoints: vec!["wss://speech.platform.bing.com".into()],
        };
        let e =
            WsClient::connect(&wrong_host, "wss://evil.example.com:443/x", 1000, 1000).unwrap_err();
        assert!(e.contains("GA1"), "{e}");
    }

    // --- LIVE probe (network; run explicitly) ---

    #[test]
    #[ignore = "live network: cargo test -p caddis-voice wss::tests::live -- --ignored --nocapture"]
    fn live_edge_tts_443_probe() {
        use crate::adapter::AudioFormat;
        use crate::edgetts_lane::EdgeTtsLane;
        use crate::lang::Lang;
        use crate::registry::{Lane, VoiceSpec};
        use crate::say::RenderLane;

        // The FULL lane path: GA1 dial + DRM + synthesis (MP3 wire) +
        // ffmpeg decode → a WAV the dispatcher could cache and play.
        let gen = GeneratorSpec {
            id: "leonas".into(),
            lane: Lane::Network,
            startup_cap_ms: 100,
            render_cap_ms: 1500,
            declared_endpoints: vec!["wss://speech.platform.bing.com".into()],
        };
        let voice = VoiceSpec {
            id: "lt-LT-LeonasNeural".into(),
            generator: "leonas".into(),
            lang: Lang::Lt,
        };
        let ffmpeg = "C:/ffmpeg/bin/ffmpeg.exe";
        assert!(
            std::path::Path::new(ffmpeg).exists(),
            "live probe needs the operator-box decoder at {ffmpeg}"
        );
        let lane = EdgeTtsLane::new(gen, 20_000, ffmpeg.into());
        let audio = lane
            .render(&voice, "Labas, čia tiesioginis ryšio patikrinimas.", 1.0)
            .expect("full lane render");
        assert!(matches!(audio.format, AudioFormat::Wav));
        assert!(
            audio.bytes.len() > 1_000,
            "suspiciously small payload: {} bytes",
            audio.bytes.len()
        );
        println!(
            "LIVE PROBE OK: {} bytes WAV (24k mono), lane elapsed {}ms (cap {}ms, over_cap={})",
            audio.bytes.len(),
            audio.elapsed_ms,
            audio.cap_ms,
            audio.over_cap
        );
    }
}
