//! hmac_rfc4231.rs — RFC 4231 test vectors for the hand-rolled HMAC-SHA256.
//!
//! These prove the in-tree HMAC implementation is correct without
//! relying on a third-party crate. The vectors are from RFC 4231
//! §4 (Test Cases for HMAC-SHA-256).
//!
//! The caddis crate is binary-only (no lib.rs), so the hmac module is
//! pulled in via include! — it is written to be self-contained (std only,
//! no crate:: paths).

#[path = "../src/hmac.rs"]
mod hmac;

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn rfc4231_case1() {
    let key = hex_decode("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b");
    let data = b"Hi There";
    let expected = "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7";
    let got = hmac::hmac_sha256(&key, data);
    assert_eq!(hex_encode(&got), expected, "RFC 4231 case 1");
}

#[test]
fn rfc4231_case2() {
    let key = b"Jefe";
    let data = b"what do ya want for nothing?";
    let expected = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";
    let got = hmac::hmac_sha256(key, data);
    assert_eq!(hex_encode(&got), expected, "RFC 4231 case 2");
}

#[test]
fn rfc4231_case4() {
    let key = hex_decode("0102030405060708090a0b0c0d0e0f10111213141516171819");
    let data = vec![0xcd_u8; 50];
    let expected = "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b";
    let got = hmac::hmac_sha256(&key, &data);
    assert_eq!(hex_encode(&got), expected, "RFC 4231 case 4");
}

#[test]
fn rfc4231_case6_key_larger_than_block() {
    // Key = 0xaa * 131 (> block size), Data = 54-byte string.
    let key = vec![0xaa_u8; 131];
    let data = b"Test Using Larger Than Block-Size Key - Hash Key First";
    let expected = "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54";
    let got = hmac::hmac_sha256(&key, data);
    assert_eq!(hex_encode(&got), expected, "RFC 4231 case 6 (key > block)");
}

#[test]
fn rfc4231_case7_key_and_data_larger_than_block() {
    // Key = 0xaa * 131, Data = 73-byte string (both > block size).
    let key = vec![0xaa_u8; 131];
    let data = b"Test Using Larger Than Block-Size Key and Larger Than One Block-Size Data";
    let expected = "c9731f25665706dab8200d9ce68fad2cbac48efc4a5f72292e4eeb81e7d29298";
    let got = hmac::hmac_sha256(&key, data);
    assert_eq!(hex_encode(&got), expected, "RFC 4231 case 7 (key + data > block)");
}

#[test]
fn sha256_empty_string() {
    let got = hmac::sha256(b"");
    let expected = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    assert_eq!(hex_encode(&got), expected, "SHA-256 of empty string");
}

#[test]
fn sha256_abc() {
    let got = hmac::sha256(b"abc");
    let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    assert_eq!(hex_encode(&got), expected, "SHA-256 of abc");
}
