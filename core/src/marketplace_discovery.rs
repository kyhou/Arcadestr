//! Eligibility for the default ADP storefront, not authorization to download or play.

use crate::canonical_hex::is_canonical_sha256;

/// Check the distribution metadata projected from a NIP-99 listing.
/// This is deliberately offline and independent of launch and acquisition policy.
pub fn has_adp_distribution_metadata(specs: &[(String, String)]) -> bool {
    specs
        .iter()
        .any(|(key, value)| key == "file_hash" && is_canonical_sha256(value))
        && specs
            .iter()
            .any(|(key, value)| key == "server" && is_distribution_url(value))
}

fn is_distribution_url(value: &str) -> bool {
    let Some((scheme, authority)) = value.split_once("://") else {
        return false;
    };
    matches!(scheme, "http" | "https")
        && !authority.is_empty()
        && !authority.starts_with(['/', '\\'])
        && url::Url::parse(value).is_ok_and(|url| {
            url.has_host() && url.username().is_empty() && url.password().is_none()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storefront_requires_a_canonical_hash_and_at_least_one_distribution_url() {
        let hash = ("file_hash".into(), "ab".repeat(32));
        let server = ("server".into(), "https://independent.example/adp".into());
        assert!(has_adp_distribution_metadata(&[
            hash.clone(),
            server.clone()
        ]));
        assert!(!has_adp_distribution_metadata(&[]));
        assert!(!has_adp_distribution_metadata(&[hash.clone()]));
        assert!(!has_adp_distribution_metadata(&[server.clone()]));
        assert!(!has_adp_distribution_metadata(&[
            ("file_hash".into(), "AB".repeat(32)),
            server.clone(),
        ]));
        for invalid in ["not a URL", "ftp://example.com", "https:///example.com"] {
            let bad_server = ("server".into(), invalid.into());
            assert!(!has_adp_distribution_metadata(&[
                hash.clone(),
                bad_server.clone()
            ]));
            assert!(has_adp_distribution_metadata(&[
                hash.clone(),
                bad_server,
                server.clone()
            ]));
        }
    }
}
