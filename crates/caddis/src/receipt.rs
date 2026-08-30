//! receipt.rs — receipt parsing, hex utilities, key management (CARD-0119).
//!
//! Extracted from rotate.rs to stay under the 280-line AGENT-LAW.

use std::env;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Split a receipt into (body, hmac) or None if malformed.
pub fn split_receipt(bytes: &[u8]) -> Option<(&[u8], [u8; 32])> {
    let sep = b"---\n";
    let pos = find_sub(bytes, sep)?;
    let body = &bytes[..pos];
    let rest = &bytes[pos + sep.len()..];
    let hex_end = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
    let hex_str = std::str::from_utf8(&rest[..hex_end]).ok()?;
    let mac_vec = decode_hex(hex_str)?;
    let mut mac = [0u8; 32];
    if mac_vec.len() != 32 {
        return None;
    }
    mac.copy_from_slice(&mac_vec);
    Some((body, mac))
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Extract a `name=value` field from a receipt body.
pub fn extract_field(body: &[u8], name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    let text = std::str::from_utf8(body).ok()?;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix(&prefix) {
            return Some(v.to_string());
        }
    }
    None
}

pub fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

pub fn hex_string(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_nibble(bytes[i])?;
        let lo = hex_nibble(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Load or create the HMAC key. CADDIS_HMAC_KEY (64 hex) overrides the file.
pub fn load_or_create_key(dir: &Path) -> Result<Vec<u8>, String> {
    let key_path = dir.join("hmac.key");
    if key_path.is_file() {
        return load_key(dir);
    }
    if let Some(key) = key_from_env() {
        return Ok(key);
    }
    let key = random_bytes(32)?;
    fs::write(&key_path, &key).map_err(|e| format!("write {}: {e}", key_path.display()))?;
    Ok(key)
}

/// Load the HMAC key. CADDIS_HMAC_KEY (64 hex) overrides the file.
pub fn load_key(dir: &Path) -> Result<Vec<u8>, String> {
    if let Some(key) = key_from_env() {
        return Ok(key);
    }
    let key_path = dir.join("hmac.key");
    fs::read(&key_path).map_err(|e| format!("read {}: {e}", key_path.display()))
}

fn key_from_env() -> Option<Vec<u8>> {
    let hex = env::var_os("CADDIS_HMAC_KEY")?;
    let hex = hex.to_string_lossy();
    let key = decode_hex(&hex)?;
    if key.len() == 32 {
        Some(key)
    } else {
        None
    }
}

fn random_bytes(n: usize) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; n];
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut f = fs::File::open("/dev/urandom").map_err(|e| format!("/dev/urandom: {e}"))?;
        f.read_exact(&mut out)
            .map_err(|e| format!("/dev/urandom read: {e}"))?;
    }
    #[cfg(windows)]
    {
        windows_random(&mut out)?;
    }
    Ok(out)
}

#[cfg(windows)]
fn windows_random(out: &mut [u8]) -> Result<(), String> {
    let n = out.len();
    let script = format!(
        "$r = New-Object byte[] {n}; \
         [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($r); \
         [BitConverter]::ToString($r).Replace('-','')"
    );
    let result = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| format!("powershell CSPRNG: {e}"))?;
    if !result.status.success() {
        return Err("powershell CSPRNG failed".into());
    }
    let hex = String::from_utf8_lossy(&result.stdout);
    let bytes =
        decode_hex(hex.trim()).ok_or_else(|| "powershell CSPRNG output not hex".to_string())?;
    if bytes.len() != n {
        return Err(format!("CSPRNG got {} bytes, want {n}", bytes.len()));
    }
    out.copy_from_slice(&bytes);
    Ok(())
}
