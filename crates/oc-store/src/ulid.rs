//! ULID ids for `session_inputs.id`.
//!
//! Layout: 48-bit millisecond timestamp (high bits) + 80 random bits,
//! Crockford base32 encoded as 26 chars, most significant first. This
//! matches the spec at <https://github.com/ulid/spec> so ids sort by
//! creation time as plain strings. No external rand crate: entropy comes
//! from `/dev/urandom`, with a pid+nanos splitmix64 fallback.

use std::fs::File;
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

/// Crockford base32 (no I, L, O, U).
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const URANDOM: &str = "/dev/urandom";
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// New ULID for "now" (wall-clock ms + 80 random bits).
pub fn new() -> String {
    let ts = now_ms();
    let rand = random_bytes();
    encode(ts, &rand)
}

/// Pure encoder: 128-bit value `(ts_ms << 80) | rand`, base32 MSB-first.
pub fn encode(ts_ms: u64, rand: &[u8; 10]) -> String {
    let value = ((ts_ms as u128) << 80) | bytes_to_u128(rand);
    let mut out = String::with_capacity(26);
    for i in 0..26 {
        let shift = 5 * (25 - i);
        let idx = ((value >> shift) & 31) as usize;
        out.push(ALPHABET[idx] as char);
    }
    out
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn bytes_to_u128(bytes: &[u8; 10]) -> u128 {
    let mut value: u128 = 0;
    for &byte in bytes {
        value = (value << 8) | byte as u128;
    }
    value
}

fn random_bytes() -> [u8; 10] {
    let mut buf = [0u8; 10];
    if let Ok(mut file) = File::open(URANDOM) {
        if file.read_exact(&mut buf).is_ok() {
            return buf;
        }
    }
    fallback_bytes()
}

/// Best-effort entropy when /dev/urandom is unreadable: splitmix64 over
/// pid and the current nanosecond clock. Uniqueness, not crypto strength.
fn fallback_bytes() -> [u8; 10] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| (d.as_secs() << 32) | u64::from(d.subsec_nanos()))
        .unwrap_or(0);
    let mut state = u64::from(std::process::id()).wrapping_mul(GOLDEN) ^ nanos;
    let mut out = [0u8; 10];
    let mut filled = 0;
    while filled < 10 {
        state = state.wrapping_add(GOLDEN);
        let z = state;
        let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        let mixed = z ^ (z >> 31);
        let take = (10 - filled).min(8);
        out[filled..filled + take].copy_from_slice(&mixed.to_le_bytes()[..take]);
        filled += take;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Independent encoder (repeated division) to cross-check `encode`.
    fn div_encode(ts: u64, rand: &[u8; 10]) -> String {
        let mut v = ((ts as u128) << 80) | bytes_to_u128(rand);
        let mut digits: Vec<u8> = Vec::new();
        while v > 0 {
            digits.push(ALPHABET[(v % 32) as usize]);
            v /= 32;
        }
        while digits.len() < 26 {
            digits.push(b'0');
        }
        digits.reverse();
        String::from_utf8(digits).expect("alphabet is ascii")
    }

    #[test]
    fn encodes_known_vectors() {
        assert_eq!(encode(0, &[0; 10]), "00000000000000000000000000");
        // +1 ms lands in the timestamp half: first 10 chars hold the 48
        // high bits, so the '1' is the 10th character, not the last.
        assert_eq!(encode(1, &[0; 10]), "00000000010000000000000000");
        let rand = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x00, 0xFF];
        // Hand-derived: value 0xdeadbeef_0123456789abcdef00ff in base32.
        assert_eq!(encode(0xDEAD_BEEF, &rand), "0003FAVFQF04HMASW9NF6YY07Z");
    }

    #[test]
    fn agrees_with_division_encoder() {
        let vectors: [(u64, [u8; 10]); 4] = [
            (0, [0; 10]),
            (1, [0; 10]),
            (
                0xDEAD_BEEF,
                [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x00, 0xFF],
            ),
            (0xFFFF_FFFF_FFFF, [0xFF; 10]),
        ];
        for (ts, rand) in vectors {
            assert_eq!(encode(ts, &rand), div_encode(ts, &rand));
            assert_eq!(encode(ts, &rand).len(), 26);
        }
    }

    #[test]
    fn sorts_monotonically_for_fixed_width_inputs() {
        let small = [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09];
        let big = [0xFF; 10];
        assert!(encode(1_700_000_000_000, &small) < encode(1_700_000_000_001, &small));
        assert!(encode(1_700_000_000_000, &small) < encode(1_700_000_000_000, &big));
    }

    #[test]
    fn new_yields_valid_crockford_ids() {
        for _ in 0..64 {
            let id = new();
            assert_eq!(id.len(), 26);
            assert!(id.bytes().all(|b| ALPHABET.contains(&b)));
        }
    }
}
