//! prober.rs — ROTATION MACHINERY slice A: the executor-family PROBE
//! transport (BUILD-QUEUE r2-rotation-machinery; brief
//! state/briefs/caddis-deliberate-rotation-brief-2026-08-28.md §6).
//!
//! The ONLY I/O module in this crate (F1 keeps the substrate pure; the
//! brief's executor-family law). A probe is a CAPABILITY LISTING —
//! `GET {base_url}/models` — never a completion: it must cost $0 on every
//! cost class (money law). Auth honors the provider's `auth_path` (a VAULT
//! PATH): the key file is read at call time and sent as a bearer; the key
//! is NEVER printed, logged, or carried in an error — secrets law.
//!
//! Laws transcribed:
//! - **Std-only organ law holds**: no crates.io dependency. HTTPS rides a
//!   schannel TLS client VENDORED from caddis-voice (`wss.rs` + the
//!   `sspi_ffi` block of `platform.rs`, live-proven over :443 there).
//!   COPIES LAW: a TLS fix lands in BOTH copies until the client
//!   graduates to a shared crate. Plain `http://` dials a bare TcpStream.
//! - **Per-probe HARD timeout** (council risk gate, brief §4): connect +
//!   total deadline; remaining time is recomputed before EVERY socket op
//!   so a stalled peer fails the probe, never the rotation lock
//!   (lock-starvation guard). Timeout = the transient class, no card.
//! - **Fail-closed, honest errors**: every failure carries a reason
//!   string with no credential material; `https` on non-Windows refuses
//!   honestly (the wss port law: a port designs its own transport, it
//!   does not inherit a pretend one).
//! - **Non-Windows**: plain-http probes (the stub-server e2e fixtures)
//!   run everywhere; the TLS dial is Windows-only by construction.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

/// Per-probe timing configuration (DATA — the rotate verb's rotation.json
/// `probe` section overrides these defaults; brief §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeCfg {
    /// TCP connect timeout.
    pub connect_timeout: Duration,
    /// HARD total deadline for one probe (resolve+dial+TLS+request+head).
    pub total_timeout: Duration,
}

impl Default for ProbeCfg {
    fn default() -> Self {
        ProbeCfg {
            connect_timeout: Duration::from_secs(10),
            total_timeout: Duration::from_secs(20),
        }
    }
}

/// One probe's wire outcome. `status` = the HTTP status code when the
/// transport answered; `error` = the honest failure reason (never carries
/// credential material). Exactly one of the two is Some on every path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeOutcome {
    pub status: Option<u16>,
    pub error: Option<String>,
}

impl ProbeOutcome {
    fn answered(status: u16) -> ProbeOutcome {
        ProbeOutcome {
            status: Some(status),
            error: None,
        }
    }
    fn failed(reason: String) -> ProbeOutcome {
        ProbeOutcome {
            status: None,
            error: Some(reason),
        }
    }
}

/// Send ONE capability-listing probe. `base_url` is the provider card's
/// URL ("https://api.example.com/v1", trailing slash tolerated); the probe
/// path is `/models`. `auth_path` empty ⇒ probe UNAUTHENTICATED (the
/// 401/403-without-auth ⇒ UNPROBEABLE law reads it in rotate.rs).
pub fn probe(base_url: &str, auth_path: &str, cfg: &ProbeCfg) -> ProbeOutcome {
    let deadline = Instant::now() + cfg.total_timeout;
    let url = match parse_url(base_url) {
        Ok(u) => u,
        Err(e) => return ProbeOutcome::failed(e),
    };
    let path = format!("{}/models", url.path.trim_end_matches('/'));
    let path = if path.starts_with("//") {
        format!("/{}", path.trim_start_matches('/'))
    } else {
        path
    };
    let bearer = if auth_path.trim().is_empty() {
        None
    } else {
        match std::fs::read_to_string(auth_path) {
            Ok(k) => {
                let k = k.trim().to_string();
                if k.is_empty() {
                    // The vault path exists but is empty — an honest local
                    // defect, not a lane verdict: transient class, visible.
                    return ProbeOutcome::failed(format!("auth file {} is empty", auth_path));
                }
                Some(k)
            }
            // Missing/unreadable key file: the probe never ran. Transient
            // class (no card) with the reason visible in the report — the
            // operator fixes the path; the lane is never Failed for it.
            Err(e) => {
                return ProbeOutcome::failed(format!("auth file unreadable: {e}"));
            }
        }
    };
    let default_port = if url.https { 443 } else { 80 };
    let host_header = if url.port == default_port {
        url.host.clone()
    } else {
        format!("{}:{}", url.host, url.port)
    };
    let mut req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: caddis-deliberate-rotate/1\r\nAccept: application/json\r\nConnection: close\r\n",
    );
    if bearer.is_some() {
        req.push_str("Authorization: Bearer ");
        req.push_str(bearer.as_deref().unwrap_or(""));
        req.push_str("\r\n");
    }
    req.push_str("\r\n");

    let mut head = Vec::with_capacity(4 * 1024);
    match dial_and_read_head(&url, req.as_bytes(), deadline, cfg, &mut head) {
        Ok(()) => {}
        Err(e) => return ProbeOutcome::failed(e),
    }
    match parse_status(&head) {
        Some(code) => ProbeOutcome::answered(code),
        None => ProbeOutcome::failed(format!(
            "malformed status line: {}",
            String::from_utf8_lossy(&head[..head.len().min(48)]).escape_default()
        )),
    }
}

// ---------------------------------------------------------------------------
// URL (the tiny slice a probe needs; not a general parser)
// ---------------------------------------------------------------------------

struct ProbeUrl {
    https: bool,
    host: String,
    port: u16,
    path: String,
}

fn parse_url(base: &str) -> Result<ProbeUrl, String> {
    let (scheme, rest) = base
        .split_once("://")
        .ok_or_else(|| format!("base_url has no scheme: {base}"))?;
    let https = match scheme {
        "https" => true,
        "http" => false,
        other => return Err(format!("unsupported scheme {other:?} (http/https only)")),
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    // Credentials in the URL are refused (secrets law — they would be
    // logged inside every report line).
    if authority.contains('@') {
        return Err("authority carries userinfo — refused".into());
    }
    let default_port = if https { 443 } else { 80 };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|_| format!("bad port in {base:?}"))?,
        ),
        None => (authority.to_string(), default_port),
    };
    if host.is_empty() {
        return Err(format!("base_url has no host: {base}"));
    }
    Ok(ProbeUrl {
        https,
        host,
        port,
        path,
    })
}

/// Parse the status code out of `HTTP/1.x NNN ...`.
fn parse_status(head: &[u8]) -> Option<u16> {
    let line_end = head
        .windows(2)
        .position(|w| w == b"\r\n")
        .unwrap_or(head.len());
    let line = &head[..line_end];
    let space1 = line.iter().position(|b| *b == b' ')?;
    let rest = &line[space1 + 1..];
    let code_end = rest
        .iter()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(rest.len());
    if code_end != 3 {
        return None;
    }
    std::str::from_utf8(&rest[..code_end]).ok()?.parse().ok()
}

// ---------------------------------------------------------------------------
// Transport: plain TCP or schannel TLS, one write + bounded head read
// ---------------------------------------------------------------------------

/// Cap on response-head bytes we will buffer (a status line + headers is
/// a few hundred bytes; anything past this is not a probe answer).
const HEAD_CAP: usize = 16 * 1024;

enum Wire {
    Plain(TcpStream),
    #[cfg(windows)]
    Tls(tls::TlsStream),
}

impl Wire {
    fn write_all(&mut self, buf: &[u8], deadline: Instant) -> Result<(), String> {
        let remaining = deadline.checked_duration_since(Instant::now());
        let Some(remaining) = remaining else {
            return Err("probe timed out (deadline, write)".into());
        };
        match self {
            Wire::Plain(s) => {
                s.set_write_timeout(Some(remaining))
                    .map_err(|e| format!("set_write_timeout: {e}"))?;
                s.write_all(buf).map_err(|e| format!("plain send: {e}"))?;
            }
            #[cfg(windows)]
            Wire::Tls(s) => {
                s.set_write_timeout(remaining);
                s.write_plain(buf)?;
            }
        }
        Ok(())
    }

    /// One bounded read. The per-op timeout is the REMAINING deadline so a
    /// slow trickle can never outlive the probe.
    fn read_some(&mut self, buf: &mut [u8], deadline: Instant) -> Result<usize, String> {
        let remaining = deadline.checked_duration_since(Instant::now());
        let Some(remaining) = remaining else {
            return Err("probe timed out (deadline, read)".into());
        };
        match self {
            Wire::Plain(s) => {
                s.set_read_timeout(Some(remaining))
                    .map_err(|e| format!("set_read_timeout: {e}"))?;
                match s.read(buf) {
                    Ok(0) => Err("connection closed before a status line".into()),
                    Ok(n) => Ok(n),
                    Err(e) => Err(format!("plain read: {e}")),
                }
            }
            #[cfg(windows)]
            Wire::Tls(s) => {
                s.set_read_timeout(remaining);
                match s.read_plain() {
                    Ok(data) => {
                        let n = data.len().min(buf.len());
                        buf[..n].copy_from_slice(&data[..n]);
                        if n == 0 {
                            return Err("tls: no plaintext".into());
                        }
                        Ok(n)
                    }
                    Err(e) => Err(format!("tls read: {e}")),
                }
            }
        }
    }
}
fn dial_and_read_head(
    url: &ProbeUrl,
    req: &[u8],
    deadline: Instant,
    cfg: &ProbeCfg,
    head: &mut Vec<u8>,
) -> Result<(), String> {
    let addr = resolve(url)?;
    let connect_budget = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| "probe timed out (deadline, connect)".to_string())?
        .min(cfg.connect_timeout);
    let mut wire = dial(url, addr, connect_budget, deadline)?;
    wire.write_all(req, deadline)?;
    loop {
        if head.len() >= HEAD_CAP {
            return Err("response head exceeds cap".into());
        }
        if head.windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(());
        }
        // A complete status line is enough to stop early on a chatty head.
        if head.contains(&b'\n') && parse_status(head).is_some() && head.ends_with(b"\r\n") {
            return Ok(());
        }
        let mut chunk = [0u8; 4 * 1024];
        match wire.read_some(&mut chunk, deadline) {
            Ok(n) => head.extend_from_slice(&chunk[..n]),
            // A server that answered and closed (Connection: close is OUR
            // request) may hand us the close before we re-checked the
            // head — if the status line is already complete, the probe
            // is answered.
            Err(e) => {
                if parse_status(head).is_some() {
                    return Ok(());
                }
                return Err(e);
            }
        }
    }
}

fn resolve(url: &ProbeUrl) -> Result<SocketAddr, String> {
    (url.host.as_str(), url.port)
        .to_socket_addrs()
        .map_err(|e| format!("cannot resolve {}: {e}", url.host))?
        .find(SocketAddr::is_ipv4)
        .ok_or_else(|| format!("no IPv4 address for {}", url.host))
}

fn dial(
    url: &ProbeUrl,
    addr: SocketAddr,
    connect_budget: Duration,
    deadline: Instant,
) -> Result<Wire, String> {
    let sock = TcpStream::connect_timeout(&addr, connect_budget)
        .map_err(|e| format!("connect {} failed: {e}", addr))?;
    sock.set_nodelay(true).ok();
    match url.https {
        false => Ok(Wire::Plain(sock)),
        #[cfg(windows)]
        true => {
            // Read timeout for the handshake legs = remaining deadline.
            let read_budget = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| "probe timed out (deadline, tls)".to_string())?;
            tls::TlsStream::connect(&url.host, url.port, connect_budget, read_budget).map(Wire::Tls)
        }
        #[cfg(not(windows))]
        true => Err("https transport is Windows-only (schannel); dial refused".into()),
    }
}

// ---------------------------------------------------------------------------
// TLS over schannel — VENDORED from caddis-voice/src/wss.rs (tls mod) +
// the sspi_ffi block of caddis-voice/src/platform.rs, live-proven over
// :443 (edge-tts WSS, 21888-byte body). COPIES LAW: a fix here lands
// there too until the client graduates to a shared crate.
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod tls {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
    use std::os::raw::{c_int, c_ulong, c_void};
    use std::time::Duration;

    // ----- sspi_ffi (ABI truth; verbatim from platform.rs) ---------------

    pub type SspiStatus = c_int; // SECURITY_STATUS / HRESULT

    pub const SEC_E_OK: SspiStatus = 0;
    pub const SEC_I_CONTINUE_NEEDED: SspiStatus = 0x00090312;
    pub const SEC_I_CONTEXT_EXPIRED: SspiStatus = 0x00090317;
    pub const SEC_E_INCOMPLETE_MESSAGE: SspiStatus = 0x80090318u32 as SspiStatus;
    pub const SEC_I_RENEGOTIATE: SspiStatus = 0x00090321;

    pub const SECPKG_CRED_OUTBOUND: c_ulong = 0x00000002;
    pub const SECURITY_NATIVE_DREP: c_ulong = 0x00000010;

    pub const SECBUFFER_VERSION: c_ulong = 0;
    pub const SECBUFFER_DATA: c_ulong = 1;
    pub const SECBUFFER_TOKEN: c_ulong = 2;
    pub const SECBUFFER_EXTRA: c_ulong = 5;
    // sspi.h: STREAM_TRAILER is 6, STREAM_HEADER is 7 — a transposed pair
    // here once cost a full SEC_E_ENCRYPT_FAILURE hunt.
    pub const SECBUFFER_STREAM_TRAILER: c_ulong = 6;
    pub const SECBUFFER_STREAM_HEADER: c_ulong = 7;
    pub const ISC_REQ_REPLAY_DETECT: u32 = 0x00000004;
    pub const ISC_REQ_SEQUENCE_DETECT: u32 = 0x00000008;
    pub const ISC_REQ_CONFIDENTIALITY: u32 = 0x00000010;
    pub const ISC_REQ_ALLOCATE_MEMORY: u32 = 0x00000100;
    pub const ISC_REQ_STREAM: u32 = 0x00008000;
    pub const ISC_REQ_INTEGRITY: u32 = 0x00010000;
    /// SECBUFFER_APPLICATION_PROTOCOLS — the ALPN list buffer type.
    pub const SECBUFFER_APPLICATION_PROTOCOLS: c_ulong = 18;
    /// SEC_APPLICATION_PROTOCOL_NEGOTIATION_EXT_ALPN.
    pub const SEC_APPLICATION_PROTOCOL_NEGOTIATION_EXT_ALPN: u32 = 2;

    /// SECPKG_ATTR_STREAM_SIZES class for QueryContextAttributes.
    pub const SECPKG_ATTR_STREAM_SIZES: c_ulong = 4;

    /// SCH_CREDENTIALS.dwVersion — the version-5 credentials struct.
    pub const SCH_CREDENTIALS_VERSION: c_ulong = 0x00000005;
    /// No client certificate: the prober authenticates nothing of ours.
    pub const SCH_CRED_NO_DEFAULT_CREDS: c_ulong = 0x00000010;
    /// SCHANNEL_SHUTDOWN control-token VALUE.
    pub const SCHANNEL_SHUTDOWN: c_ulong = 0x00000001;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct SecHandle {
        pub dw_lower: usize,
        pub dw_upper: usize,
    }

    impl SecHandle {
        pub const fn null() -> Self {
            Self {
                dw_lower: 0,
                dw_upper: 0,
            }
        }
        pub fn is_null(&self) -> bool {
            self.dw_lower == 0 && self.dw_upper == 0
        }
    }

    #[repr(C)]
    pub struct SecBuffer {
        pub cb_buffer: c_ulong,
        pub buffer_type: c_ulong,
        pub pv_buffer: *mut c_void,
    }

    #[repr(C)]
    pub struct SecBufferDesc {
        pub ul_version: c_ulong,
        pub c_buffers: c_ulong,
        pub p_buffers: *mut SecBuffer,
    }

    /// SCH_CREDENTIALS (schannel.h, dwVersion 5). Zeroed tail = default
    /// TLS parameters, no client certs. Automatic chain validation stays
    /// ON: the host name in the ISC target name drives SNI + validation.
    #[repr(C)]
    pub struct SchCredentials {
        pub dw_version: c_ulong,
        pub dw_cred_format: c_ulong,
        pub c_creds: c_ulong,
        pub pa_cred: *const *const c_void,
        pub h_root_store: *mut c_void,
        pub c_mappers: c_ulong,
        pub aph_mappers: *const *const c_void,
        pub dw_session_lifespan: c_ulong,
        pub dw_flags: c_ulong,
        pub c_tls_parameters: c_ulong,
        pub p_tls_parameters: *const c_void,
    }

    /// SecPkgContext_StreamSizes.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct StreamSizes {
        pub cb_header: c_ulong,
        pub cb_trailer: c_ulong,
        pub cb_maximum_message: c_ulong,
        pub c_buffers: c_ulong,
        pub cb_block_size: c_ulong,
    }

    #[link(name = "secur32")]
    extern "system" {
        pub fn AcquireCredentialsHandleW(
            psz_principal: *const u16,
            psz_package: *const u16,
            f_credential_use: c_ulong,
            pv_logon_id: *const c_void,
            p_auth_data: *const c_void,
            p_get_key_fn: *const c_void,
            pv_get_key_argument: *const c_void,
            ph_credential: *mut SecHandle,
            pts_expiry: *mut i64,
        ) -> SspiStatus;
        pub fn FreeCredentialsHandle(ph_credential: *mut SecHandle) -> SspiStatus;
        pub fn InitializeSecurityContextW(
            ph_credential: *const SecHandle,
            ph_context: *const SecHandle,
            psz_target_name: *const u16,
            f_context_req: u32,
            reserved1: c_ulong,
            target_data_rep: c_ulong,
            p_input: *const SecBufferDesc,
            reserved2: c_ulong,
            ph_new_context: *mut SecHandle,
            p_output: *mut SecBufferDesc,
            pf_context_attr: *mut u32,
            pts_expiry: *mut i64,
        ) -> SspiStatus;
        pub fn DeleteSecurityContext(ph_context: *const SecHandle) -> SspiStatus;
        pub fn ApplyControlToken(
            ph_context: *const SecHandle,
            p_input: *const SecBufferDesc,
        ) -> SspiStatus;
        pub fn QueryContextAttributesW(
            ph_context: *const SecHandle,
            ul_attribute: c_ulong,
            p_buffer: *mut c_void,
        ) -> SspiStatus;
        pub fn EncryptMessage(
            ph_context: *const SecHandle,
            f_qop: c_ulong,
            p_message: *mut SecBufferDesc,
            message_seq_no: c_ulong,
        ) -> SspiStatus;
        pub fn DecryptMessage(
            ph_context: *const SecHandle,
            p_message: *mut SecBufferDesc,
            message_seq_no: c_ulong,
            pf_qop: *mut c_ulong,
        ) -> SspiStatus;
        pub fn FreeContextBuffer(pv_context_buffer: *mut c_void) -> SspiStatus;
    }

    // ----- TlsStream (adapted from wss.rs; WSS framing dropped, the
    // TLS record engine kept verbatim) ----------------------------------

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
        /// Ciphertext read from the socket, not yet consumed by
        /// DecryptMessage.
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
        /// first handshake read on; the write timeout is set once (writes
        /// are short request flights).
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
            sock.set_write_timeout(Some(read_timeout))
                .map_err(|e| format!("tls: set_write_timeout: {e}"))?;
            sock.set_nodelay(true).ok();

            let host_w: Vec<u16> = host.encode_utf16().chain(std::iter::once(0)).collect();

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
            // client→server app keys only after consuming it. Bounded drain:
            // ONE short read into the cipher buffer, every error ignored
            // (nothing pending is the common case, not a defect).
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
        /// context already exists. Input desc is [TOKEN, EMPTY, ALPN].
        fn isc_loop(&mut self, mut pending: Vec<u8>, resumed: bool) -> Result<(), String> {
            let mut first = !resumed;
            let mut attrs: u32 = 0;
            // ALPN "http/1.1" — the same wire the proven wss client sends;
            // for a plain HTTPS GET it is exactly the right negotiation.
            // SEC_APPLICATION_PROTOCOLS byte shape:
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
                        // Leftover server bytes become the first ciphertext.
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

        pub fn set_write_timeout(&mut self, d: Duration) {
            let _ = self.sock.set_write_timeout(Some(d));
        }

        pub fn set_read_timeout(&mut self, d: Duration) {
            let _ = self.sock.set_read_timeout(Some(d));
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
                SEC_I_RENEGOTIATE => Ok(StepOutcome::Renegotiate { extra_off }),
                SEC_E_INCOMPLETE_MESSAGE => Ok(StepOutcome::Incomplete),
                SEC_I_CONTEXT_EXPIRED => Err("tls: server sent close notify".into()),
                other => Err(format!("tls: decrypt {}", status_hex(other))),
            }
        }

        /// Best-effort close_notify: ApplyControlToken(SCHANNEL_SHUTDOWN)
        /// + one more ISC produces the shutdown record; send and tear down.
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

#[cfg(test)]
#[path = "prober_tests.rs"]
mod tests;
