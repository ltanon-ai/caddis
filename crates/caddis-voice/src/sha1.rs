//! sha1.rs — SHA-1 (FIPS 180-1), std-only, self-contained.
//!
//! Needed by exactly ONE thing: the WebSocket handshake's
//! `Sec-WebSocket-Acact` verification (`base64(SHA-1(key + GUID))`,
//! RFC 6455 §4.2.2) in wss.rs. Not used anywhere security-critical —
//! SHA-1 is collision-broken and this organ never leans on it for
//! integrity; the WS accept check is a server-liveness echo, and the
//! DRM token lane stays on SHA-256 (sha256.rs).
//!
//! Vendored by organ law, sha256.rs precedent: any FIX lands in BOTH
//! copies until the primitive graduates to caddis-core.

const K: [u32; 4] = [0x5a827999, 0x6ed9eba1, 0x8f1bbcdc, 0xca62c1d6];

const H0: [u32; 5] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0];

/// SHA-1 digest of `data`.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut h = H0;
    let mut w = [0u32; 80];
    for block in msg.chunks_exact(64) {
        for (i, word) in w[..16].iter_mut().enumerate() {
            let s = i * 4;
            *word = u32::from_be_bytes([block[s], block[s + 1], block[s + 2], block[s + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i / 20 {
                0 => ((b & c) | ((!b) & d), K[0]),
                1 => (b ^ c ^ d, K[1]),
                2 => ((b & c) | (b & d) | (c & d), K[2]),
                _ => (b ^ c ^ d, K[3]),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(d: &[u8]) -> String {
        d.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn fips_vectors() {
        // FIPS 180-1 §7 examples + the empty-string digest.
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn million_a_vector() {
        // "a" × 1,000,000 → 34aa973cd4c4daa4f61eeb2bdbad27316534016f
        let data = vec![b'a'; 1_000_000];
        assert_eq!(
            hex(&sha1(&data)),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
    }
}
