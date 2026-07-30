//! Blossom protocol types and validation used by media upload clients.

use base64::Engine as _;
use nostr::{Event, EventBuilder, Kind, PublicKey, Tag, TagKind, Timestamp, UnsignedEvent};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::{Host, Url};

use crate::store_page_content_policy::validate_store_page_url;

pub const BLOSSOM_UPLOAD_AUTHORIZATION_KIND: u16 = 24_242;
pub const BLOSSOM_USER_SERVER_LIST_KIND: u16 = 10_063;
pub const BLOSSOM_IMAGE_MAX_BYTES: u64 = 20 * 1024 * 1024;
pub const BLOSSOM_VIDEO_MAX_BYTES: u64 = 500 * 1024 * 1024;

const MAX_AUTHORIZATION_CONTENT_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlossomServerOriginPolicy {
    HttpsOnly,
    AllowHttpLoopback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlossomServerOrigin {
    normalized: String,
    authorization_domain: Option<String>,
}

impl BlossomServerOrigin {
    pub fn as_str(&self) -> &str {
        &self.normalized
    }

    pub fn authorization_domain(&self) -> Option<&str> {
        self.authorization_domain.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadAuthorizationInput {
    pub publisher: PublicKey,
    pub sha256: String,
    pub created_at: u64,
    pub expiration: u64,
    pub server: Option<BlossomServerOrigin>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlossomBlobDescriptor {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    #[serde(rename = "type")]
    pub mime_type: String,
    pub uploaded: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlossomBlobExpectation<'a> {
    pub sha256: &'a str,
    pub size: u64,
    pub mime_type: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlossomMimePolicy {
    pub mime_type: &'static str,
    pub media_type: &'static str,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BlossomError {
    #[error("SHA-256 must contain exactly 64 lowercase hexadecimal characters")]
    InvalidSha256,
    #[error("upload authorization expiration must be later than created_at")]
    InvalidExpiration,
    #[error("upload authorization content must be non-empty, bounded human-readable text")]
    InvalidAuthorizationContent,
    #[error("invalid Blossom server origin: {0}")]
    InvalidServerOrigin(&'static str),
    #[error("unsupported Blossom media MIME type: {0}")]
    UnsupportedMimeType(String),
    #[error("Blossom media size must be greater than zero")]
    InvalidSize,
    #[error("Blossom media size {size} exceeds the {max} byte limit for {mime_type}")]
    SizeTooLarge {
        mime_type: String,
        size: u64,
        max: u64,
    },
    #[error("failed to serialize Blossom authorization event: {0}")]
    AuthorizationSerialization(String),
    #[error("invalid Blossom blob descriptor JSON: {0}")]
    InvalidDescriptorJson(String),
    #[error("Blossom blob descriptor hash does not match the uploaded blob")]
    DescriptorHashMismatch,
    #[error("Blossom blob descriptor size does not match the uploaded blob")]
    DescriptorSizeMismatch,
    #[error("Blossom blob descriptor MIME type does not match the uploaded blob")]
    DescriptorMimeMismatch,
    #[error("Blossom blob descriptor URL is unsafe or does not contain the uploaded hash")]
    InvalidDescriptorUrl,
    #[error("Blossom blob descriptor uploaded timestamp is invalid")]
    InvalidUploadedTimestamp,
}

pub fn is_lowercase_sha256_hex(hash: &str) -> bool {
    crate::is_sha256_hex(hash)
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn blossom_mime_policy(mime_type: &str) -> Result<BlossomMimePolicy, BlossomError> {
    let policy = match mime_type {
        "image/jpeg" => BlossomMimePolicy {
            mime_type: "image/jpeg",
            media_type: "image",
            max_bytes: BLOSSOM_IMAGE_MAX_BYTES,
        },
        "image/png" => BlossomMimePolicy {
            mime_type: "image/png",
            media_type: "image",
            max_bytes: BLOSSOM_IMAGE_MAX_BYTES,
        },
        "image/webp" => BlossomMimePolicy {
            mime_type: "image/webp",
            media_type: "image",
            max_bytes: BLOSSOM_IMAGE_MAX_BYTES,
        },
        "video/mp4" => BlossomMimePolicy {
            mime_type: "video/mp4",
            media_type: "video",
            max_bytes: BLOSSOM_VIDEO_MAX_BYTES,
        },
        "video/webm" => BlossomMimePolicy {
            mime_type: "video/webm",
            media_type: "video",
            max_bytes: BLOSSOM_VIDEO_MAX_BYTES,
        },
        _ => return Err(BlossomError::UnsupportedMimeType(mime_type.to_string())),
    };
    Ok(policy)
}

pub fn validate_blossom_media(
    mime_type: &str,
    size: u64,
) -> Result<BlossomMimePolicy, BlossomError> {
    let policy = blossom_mime_policy(mime_type)?;
    if size == 0 {
        return Err(BlossomError::InvalidSize);
    }
    if size > policy.max_bytes {
        return Err(BlossomError::SizeTooLarge {
            mime_type: mime_type.to_string(),
            size,
            max: policy.max_bytes,
        });
    }
    Ok(policy)
}

pub fn validate_blossom_server_origin(
    value: &str,
    policy: BlossomServerOriginPolicy,
) -> Result<BlossomServerOrigin, BlossomError> {
    if value.is_empty()
        || value.trim() != value
        || value.bytes().any(|byte| byte.is_ascii_control())
        || value.contains('\\')
    {
        return Err(BlossomError::InvalidServerOrigin("malformed URL"));
    }

    let parsed =
        Url::parse(value).map_err(|_| BlossomError::InvalidServerOrigin("malformed URL"))?;
    let scheme_suffix = value
        .get(parsed.scheme().len()..)
        .ok_or(BlossomError::InvalidServerOrigin("malformed URL"))?;
    if !scheme_suffix.starts_with("://") || scheme_suffix[3..].starts_with('/') {
        return Err(BlossomError::InvalidServerOrigin("malformed URL"));
    }
    if parsed.cannot_be_a_base() || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(BlossomError::InvalidServerOrigin(
            "credentials and non-hierarchical URLs are forbidden",
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() || parsed.path() != "/" {
        return Err(BlossomError::InvalidServerOrigin(
            "query, fragment, and non-root paths are forbidden",
        ));
    }

    let host = parsed
        .host()
        .ok_or(BlossomError::InvalidServerOrigin("host is required"))?;
    let loopback = match host {
        Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
    };
    match parsed.scheme() {
        "https" => {}
        "http" if policy == BlossomServerOriginPolicy::AllowHttpLoopback && loopback => {}
        "http" => {
            return Err(BlossomError::InvalidServerOrigin(
                "HTTP is allowed only for explicit loopback development origins",
            ))
        }
        _ => return Err(BlossomError::InvalidServerOrigin("HTTPS is required")),
    }

    let authorization_domain = match host {
        Host::Domain(domain) if valid_domain(domain) => Some(domain.to_ascii_lowercase()),
        Host::Domain(_) => return Err(BlossomError::InvalidServerOrigin("malformed host")),
        Host::Ipv4(_) | Host::Ipv6(_) => None,
    };

    Ok(BlossomServerOrigin {
        normalized: parsed.to_string(),
        authorization_domain,
    })
}

pub fn build_upload_authorization(
    input: &UploadAuthorizationInput,
) -> Result<UnsignedEvent, BlossomError> {
    if !is_lowercase_sha256_hex(&input.sha256) {
        return Err(BlossomError::InvalidSha256);
    }
    if input.expiration <= input.created_at {
        return Err(BlossomError::InvalidExpiration);
    }
    if input.content.is_empty()
        || input.content.trim() != input.content
        || input.content.chars().count() > MAX_AUTHORIZATION_CONTENT_CHARS
        || input.content.chars().any(char::is_control)
    {
        return Err(BlossomError::InvalidAuthorizationContent);
    }

    let mut tags = vec![
        Tag::custom(TagKind::Custom("t".into()), ["upload"]),
        Tag::custom(
            TagKind::Custom("expiration".into()),
            [input.expiration.to_string()],
        ),
        Tag::custom(TagKind::Custom("x".into()), [input.sha256.clone()]),
    ];
    if let Some(domain) = input
        .server
        .as_ref()
        .and_then(BlossomServerOrigin::authorization_domain)
    {
        tags.push(Tag::custom(
            TagKind::Custom("server".into()),
            [domain.to_string()],
        ));
    }

    Ok(EventBuilder::new(
        Kind::Custom(BLOSSOM_UPLOAD_AUTHORIZATION_KIND),
        input.content.clone(),
    )
    .tags(tags)
    .custom_created_at(Timestamp::from_secs(input.created_at))
    .build(input.publisher))
}

pub fn encode_blossom_authorization_header(event: &Event) -> Result<String, BlossomError> {
    let json = serde_json::to_vec(event)
        .map_err(|error| BlossomError::AuthorizationSerialization(error.to_string()))?;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json);
    Ok(format!("Nostr {encoded}"))
}

pub fn parse_blob_descriptor(value: &str) -> Result<BlossomBlobDescriptor, BlossomError> {
    serde_json::from_str(value)
        .map_err(|error| BlossomError::InvalidDescriptorJson(error.to_string()))
}

pub fn validate_blob_descriptor(
    mut descriptor: BlossomBlobDescriptor,
    expected: BlossomBlobExpectation<'_>,
) -> Result<BlossomBlobDescriptor, BlossomError> {
    if !is_lowercase_sha256_hex(&descriptor.sha256) || descriptor.sha256 != expected.sha256 {
        return Err(BlossomError::DescriptorHashMismatch);
    }
    validate_blossom_media(expected.mime_type, expected.size)?;
    if descriptor.size != expected.size {
        return Err(BlossomError::DescriptorSizeMismatch);
    }
    validate_blossom_media(&descriptor.mime_type, descriptor.size)?;
    if descriptor.mime_type != expected.mime_type {
        return Err(BlossomError::DescriptorMimeMismatch);
    }
    if descriptor.uploaded == 0 {
        return Err(BlossomError::InvalidUploadedTimestamp);
    }

    let url =
        validate_store_page_url(&descriptor.url).map_err(|_| BlossomError::InvalidDescriptorUrl)?;
    if last_lowercase_sha256_hex(&url) != Some(expected.sha256) {
        return Err(BlossomError::InvalidDescriptorUrl);
    }
    descriptor.url = url;
    Ok(descriptor)
}

pub fn blossom_blob_url_matches_sha256(url: &str, sha256: &str) -> bool {
    is_lowercase_sha256_hex(sha256) && last_lowercase_sha256_hex(url) == Some(sha256)
}

fn valid_domain(domain: &str) -> bool {
    if domain.eq_ignore_ascii_case("localhost") {
        return true;
    }
    domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn last_lowercase_sha256_hex(value: &str) -> Option<&str> {
    value
        .as_bytes()
        .windows(64)
        .enumerate()
        .filter(|(_, bytes)| {
            bytes
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        })
        .filter_map(|(start, _)| value.get(start..start + 64))
        .last()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use nostr::Keys;
    use serde_json::Value;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn authorization_input() -> UploadAuthorizationInput {
        let keys = Keys::generate();
        UploadAuthorizationInput {
            publisher: keys.public_key(),
            sha256: HASH.to_string(),
            created_at: 1_700_000_000,
            expiration: 1_700_000_600,
            server: Some(
                validate_blossom_server_origin(
                    "https://Media.Example",
                    BlossomServerOriginPolicy::HttpsOnly,
                )
                .expect("server origin"),
            ),
            content: "Upload media blob".to_string(),
        }
    }

    fn descriptor_json(extra: &str) -> String {
        format!(
            r#"{{"url":"https://cdn.example/{HASH}.webp","sha256":"{HASH}","size":42,"type":"image/webp","uploaded":1700000000{extra}}}"#
        )
    }

    fn expectation() -> BlossomBlobExpectation<'static> {
        BlossomBlobExpectation {
            sha256: HASH,
            size: 42,
            mime_type: "image/webp",
        }
    }

    #[test]
    fn upload_authorization_has_required_tags_and_normalized_server() {
        let unsigned = build_upload_authorization(&authorization_input()).expect("authorization");
        let tags = unsigned
            .tags
            .iter()
            .map(|tag| tag.as_slice())
            .collect::<Vec<_>>();

        assert_eq!(
            unsigned.kind,
            Kind::Custom(BLOSSOM_UPLOAD_AUTHORIZATION_KIND)
        );
        assert_eq!(unsigned.content, "Upload media blob");
        assert!(tags.iter().any(|tag| *tag == ["t", "upload"]));
        assert!(tags.iter().any(|tag| *tag == ["expiration", "1700000600"]));
        assert!(tags.iter().any(|tag| *tag == ["x", HASH]));
        assert!(tags.iter().any(|tag| *tag == ["server", "media.example"]));
    }

    #[test]
    fn upload_authorization_rejects_invalid_hash_and_expiration() {
        let mut input = authorization_input();
        input.sha256 = HASH.to_ascii_uppercase();
        assert_eq!(
            build_upload_authorization(&input),
            Err(BlossomError::InvalidSha256)
        );

        input.sha256 = HASH.to_string();
        input.expiration = input.created_at;
        assert_eq!(
            build_upload_authorization(&input),
            Err(BlossomError::InvalidExpiration)
        );
    }

    #[tokio::test]
    async fn authorization_header_uses_base64url_without_padding() {
        let keys = Keys::generate();
        let mut input = authorization_input();
        input.publisher = keys.public_key();
        let event = build_upload_authorization(&input)
            .expect("authorization")
            .sign(&keys)
            .await
            .expect("signed event");
        let header = encode_blossom_authorization_header(&event).expect("header");
        let encoded = header.strip_prefix("Nostr ").expect("scheme");

        assert!(!encoded.contains('='));
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .expect("base64url");
        let decoded: Event = serde_json::from_slice(&decoded).expect("event JSON");
        assert_eq!(decoded.id, event.id);
    }

    #[test]
    fn descriptor_parses_valid_response_and_ignores_unknown_fields() {
        let descriptor =
            parse_blob_descriptor(&descriptor_json(",\"magnet\":\"ignored\"")).expect("descriptor");
        let validated = validate_blob_descriptor(descriptor, expectation()).expect("valid");

        assert_eq!(validated.sha256, HASH);
        assert_eq!(validated.mime_type, "image/webp");
    }

    #[test]
    fn descriptor_rejects_hash_size_and_mime_mismatches() {
        let mut descriptor = parse_blob_descriptor(&descriptor_json("")).expect("descriptor");
        descriptor.sha256 = "a".repeat(64);
        assert_eq!(
            validate_blob_descriptor(descriptor, expectation()),
            Err(BlossomError::DescriptorHashMismatch)
        );

        let mut descriptor = parse_blob_descriptor(&descriptor_json("")).expect("descriptor");
        descriptor.size = 43;
        assert_eq!(
            validate_blob_descriptor(descriptor, expectation()),
            Err(BlossomError::DescriptorSizeMismatch)
        );

        let mut descriptor = parse_blob_descriptor(&descriptor_json("")).expect("descriptor");
        descriptor.mime_type = "image/png".to_string();
        assert_eq!(
            validate_blob_descriptor(descriptor, expectation()),
            Err(BlossomError::DescriptorMimeMismatch)
        );
    }

    #[test]
    fn descriptor_rejects_unsafe_url_and_invalid_uploaded_timestamp() {
        let mut descriptor = parse_blob_descriptor(&descriptor_json("")).expect("descriptor");
        descriptor.url = format!("http://cdn.example/{HASH}.webp");
        assert_eq!(
            validate_blob_descriptor(descriptor, expectation()),
            Err(BlossomError::InvalidDescriptorUrl)
        );

        let mut descriptor = parse_blob_descriptor(&descriptor_json("")).expect("descriptor");
        descriptor.uploaded = 0;
        assert_eq!(
            validate_blob_descriptor(descriptor, expectation()),
            Err(BlossomError::InvalidUploadedTimestamp)
        );
    }

    #[test]
    fn mime_policy_accepts_allowed_boundaries_and_rejects_others() {
        for (mime_type, max) in [
            ("image/jpeg", BLOSSOM_IMAGE_MAX_BYTES),
            ("image/png", BLOSSOM_IMAGE_MAX_BYTES),
            ("image/webp", BLOSSOM_IMAGE_MAX_BYTES),
            ("video/mp4", BLOSSOM_VIDEO_MAX_BYTES),
            ("video/webm", BLOSSOM_VIDEO_MAX_BYTES),
        ] {
            assert_eq!(
                validate_blossom_media(mime_type, 0),
                Err(BlossomError::InvalidSize)
            );
            assert!(validate_blossom_media(mime_type, 1).is_ok());
            assert!(validate_blossom_media(mime_type, max).is_ok());
            assert!(matches!(
                validate_blossom_media(mime_type, max + 1),
                Err(BlossomError::SizeTooLarge { .. })
            ));
        }

        assert!(matches!(
            validate_blossom_media("image/gif", 1),
            Err(BlossomError::UnsupportedMimeType(_))
        ));
        assert!(matches!(
            validate_blossom_media("application/octet-stream", 1),
            Err(BlossomError::UnsupportedMimeType(_))
        ));
    }

    #[test]
    fn server_origin_policy_is_structural_and_explicit_about_loopback_http() {
        let origin = validate_blossom_server_origin(
            "https://Blossom.Example:443",
            BlossomServerOriginPolicy::HttpsOnly,
        )
        .expect("HTTPS origin");
        assert_eq!(origin.as_str(), "https://blossom.example/");
        assert_eq!(origin.authorization_domain(), Some("blossom.example"));

        assert!(validate_blossom_server_origin(
            "http://127.0.0.1:3000",
            BlossomServerOriginPolicy::HttpsOnly,
        )
        .is_err());
        assert!(validate_blossom_server_origin(
            "http://127.0.0.1:3000",
            BlossomServerOriginPolicy::AllowHttpLoopback,
        )
        .is_ok());
        for unsafe_origin in [
            "https://user@example.com",
            "https://example.com/upload",
            "https://example.com?mode=upload",
            "https://example.com/#fragment",
            "https:///example.com",
        ] {
            assert!(validate_blossom_server_origin(
                unsafe_origin,
                BlossomServerOriginPolicy::HttpsOnly,
            )
            .is_err());
        }
    }

    #[test]
    fn descriptor_requires_all_protocol_fields() {
        let value: Value = serde_json::from_str(&descriptor_json("")).expect("JSON");
        for field in ["url", "sha256", "size", "type", "uploaded"] {
            let mut missing = value.clone();
            missing.as_object_mut().expect("object").remove(field);
            assert!(parse_blob_descriptor(&missing.to_string()).is_err());
        }
    }
}
