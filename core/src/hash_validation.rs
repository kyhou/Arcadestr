/// Returns whether `hash` is an exact SHA-256 hexadecimal digest.
pub fn is_sha256_hex(hash: &str) -> bool {
    hash.len() == 64 && hash.as_bytes().iter().all(u8::is_ascii_hexdigit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_exact_ascii_sha256_hex() {
        assert!(is_sha256_hex(
            "0123456789abcdefABCDEF0123456789abcdefABCDEF0123456789abcdefABCD"
        ));
    }

    #[test]
    fn rejects_wrong_length_non_hex_and_non_ascii_values() {
        for hash in [
            "",
            "abc123",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0",
            "g123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "é123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde",
        ] {
            assert!(!is_sha256_hex(hash), "unexpectedly accepted {hash:?}");
        }
    }
}
