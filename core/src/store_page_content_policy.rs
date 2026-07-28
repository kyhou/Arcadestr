use pulldown_cmark::{html, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::Serialize;
use thiserror::Error;
use url::Url;

// These limits fit typical 128 KiB relay envelopes while leaving room for event metadata.
pub const MAX_EVENT_BYTES: usize = 128 * 1024;
pub const MAX_EVENT_CONTENT_BYTES: usize = 96 * 1024;
pub const MAX_TITLE_CHARS: usize = 120;
pub const MAX_SUMMARY_CHARS: usize = 300;
pub const MAX_MARKDOWN_SOURCE_BYTES: usize = 64 * 1024;
pub const MAX_MARKDOWN_OUTPUT_BYTES: usize = 96 * 1024;
pub const MAX_SECTION_MARKDOWN_SOURCE_BYTES: usize = 4 * 1024;
pub const MAX_SECTION_MARKDOWN_OUTPUT_BYTES: usize = 8 * 1024;
pub const MAX_ASSOCIATED_LISTINGS: usize = 16;
pub const MAX_MEDIA_ITEMS: usize = 40;
pub const MAX_SCREENSHOTS: usize = 24;
pub const MAX_TRAILERS: usize = 4;
pub const MAX_FEATURE_SECTIONS: usize = 12;
pub const MAX_LANGUAGES: usize = 64;
pub const MAX_ACCESSIBILITY_ENTRIES: usize = 32;
pub const MAX_EXTERNAL_LINKS: usize = 12;
pub const MAX_GENRES: usize = 8;
pub const MAX_FEATURES: usize = 16;
pub const MAX_URL_BYTES: usize = 2_048;
pub const MAX_TEXT_FIELD_CHARS: usize = 4_096;
pub const MAX_IDENTIFIER_CHARS: usize = 128;
pub const MAX_MARKDOWN_NESTING_DEPTH: usize = 12;
pub const MAX_MARKDOWN_LINKS: usize = 64;
pub const MAX_MARKDOWN_IMAGES: usize = 16;
pub const MAX_MARKDOWN_HEADING_LEVEL: u8 = 3;

pub const ALLOWED_DIRECT_VIDEO_EXTENSIONS: &[&str] = &["mp4", "webm"];
pub const STORE_PAGE_CONTENT_POLICY_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ContentPolicyError {
    #[error("{field} exceeds limit {max}")]
    LimitExceeded { field: String, max: usize },
    #[error("invalid Store Page URL")]
    InvalidUrl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum MarkdownDiagnostic {
    RawHtmlRemoved,
    UnsafeLinkRemoved,
    UnsafeImageRemoved,
}

/// HTML can only be constructed by the Store Page Markdown sanitizer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct SanitizedMarkdown(String);

impl SanitizedMarkdown {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn validate_store_page_url(value: &str) -> Result<String, ContentPolicyError> {
    if value.is_empty()
        || value.len() > MAX_URL_BYTES
        || value.trim() != value
        || value.bytes().any(|byte| byte.is_ascii_control())
        || value.contains('\\')
    {
        return Err(ContentPolicyError::InvalidUrl);
    }
    let parsed = Url::parse(value).map_err(|_| ContentPolicyError::InvalidUrl)?;
    if parsed.scheme() != "https"
        || parsed.cannot_be_a_base()
        || parsed.host_str().map_or(true, str::is_empty)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(ContentPolicyError::InvalidUrl);
    }

    // Reject ambiguous forms such as `https:///host` that WHATWG parsing normalizes.
    let scheme_suffix = value
        .get(parsed.scheme().len()..)
        .ok_or(ContentPolicyError::InvalidUrl)?;
    if !scheme_suffix.starts_with("://") || scheme_suffix[3..].starts_with('/') {
        return Err(ContentPolicyError::InvalidUrl);
    }
    Ok(parsed.to_string())
}

pub fn is_allowed_direct_video_url(value: &str) -> bool {
    let Ok(canonical) = validate_store_page_url(value) else {
        return false;
    };
    let Ok(parsed) = Url::parse(&canonical) else {
        return false;
    };
    parsed
        .path_segments()
        .and_then(Iterator::last)
        .and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))
        .is_some_and(|extension| {
            ALLOWED_DIRECT_VIDEO_EXTENSIONS
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        })
}

pub fn sanitize_markdown(
    source: &str,
    source_limit: usize,
    output_limit: usize,
    field: &str,
) -> Result<(SanitizedMarkdown, Vec<MarkdownDiagnostic>), ContentPolicyError> {
    if source.len() > source_limit {
        return Err(ContentPolicyError::LimitExceeded {
            field: field.to_string(),
            max: source_limit,
        });
    }

    let mut events = Vec::new();
    let mut diagnostics = Vec::new();
    let mut depth = 0usize;
    let mut link_count = 0usize;
    let mut image_count = 0usize;
    let mut suppressed_link_ends = 0usize;
    let mut suppressed_image_ends = 0usize;

    for event in Parser::new_ext(source, Options::empty()) {
        match event {
            Event::Start(tag) => {
                depth += 1;
                if depth > MAX_MARKDOWN_NESTING_DEPTH {
                    return Err(ContentPolicyError::LimitExceeded {
                        field: format!("{field} nesting depth"),
                        max: MAX_MARKDOWN_NESTING_DEPTH,
                    });
                }
                match tag {
                    Tag::HtmlBlock => {
                        push_once(&mut diagnostics, MarkdownDiagnostic::RawHtmlRemoved);
                    }
                    Tag::Heading { level, .. }
                        if heading_level(level) > MAX_MARKDOWN_HEADING_LEVEL =>
                    {
                        return Err(ContentPolicyError::LimitExceeded {
                            field: format!("{field} heading level"),
                            max: MAX_MARKDOWN_HEADING_LEVEL as usize,
                        });
                    }
                    Tag::Link {
                        link_type,
                        dest_url,
                        title,
                        id,
                    } => {
                        link_count += 1;
                        if link_count > MAX_MARKDOWN_LINKS {
                            return Err(ContentPolicyError::LimitExceeded {
                                field: format!("{field} links"),
                                max: MAX_MARKDOWN_LINKS,
                            });
                        }
                        match validate_store_page_url(dest_url.as_ref()) {
                            Ok(url) => events.push(Event::Start(Tag::Link {
                                link_type,
                                dest_url: url.into(),
                                title,
                                id,
                            })),
                            Err(_) => {
                                suppressed_link_ends += 1;
                                push_once(&mut diagnostics, MarkdownDiagnostic::UnsafeLinkRemoved);
                            }
                        }
                    }
                    Tag::Image {
                        link_type,
                        dest_url,
                        title,
                        id,
                    } => {
                        image_count += 1;
                        if image_count > MAX_MARKDOWN_IMAGES {
                            return Err(ContentPolicyError::LimitExceeded {
                                field: format!("{field} images"),
                                max: MAX_MARKDOWN_IMAGES,
                            });
                        }
                        match validate_store_page_url(dest_url.as_ref()) {
                            Ok(url) => events.push(Event::Start(Tag::Image {
                                link_type,
                                dest_url: url.into(),
                                title,
                                id,
                            })),
                            Err(_) => {
                                suppressed_image_ends += 1;
                                push_once(&mut diagnostics, MarkdownDiagnostic::UnsafeImageRemoved);
                            }
                        }
                    }
                    other => events.push(Event::Start(other)),
                }
            }
            Event::End(end) => {
                depth = depth.saturating_sub(1);
                match end {
                    TagEnd::HtmlBlock => {}
                    TagEnd::Link if suppressed_link_ends > 0 => suppressed_link_ends -= 1,
                    TagEnd::Image if suppressed_image_ends > 0 => suppressed_image_ends -= 1,
                    other => events.push(Event::End(other)),
                }
            }
            Event::Html(_) | Event::InlineHtml(_) => {
                push_once(&mut diagnostics, MarkdownDiagnostic::RawHtmlRemoved);
            }
            other => events.push(other),
        }
    }

    let mut output = String::with_capacity(source.len());
    html::push_html(&mut output, events.into_iter());
    output = output.replace(
        "<a href=",
        "<a target=\"_blank\" rel=\"noopener noreferrer\" href=",
    );
    if output.len() > output_limit {
        return Err(ContentPolicyError::LimitExceeded {
            field: format!("{field} sanitized output"),
            max: output_limit,
        });
    }
    Ok((SanitizedMarkdown(output), diagnostics))
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn push_once(diagnostics: &mut Vec<MarkdownDiagnostic>, diagnostic: MarkdownDiagnostic) {
    if !diagnostics.contains(&diagnostic) {
        diagnostics.push(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_page_url_policy_accepts_only_structural_credential_free_https() {
        for valid in [
            "https://example.com/image.webp",
            "HTTPS://example.com/path?q=value#fragment",
        ] {
            assert!(validate_store_page_url(valid).is_ok(), "{valid}");
        }

        for invalid in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "data:image/png;base64,AAAA",
            "file:///tmp/image.png",
            "blob:https://example.com/id",
            "tauri://localhost/file",
            "asset://localhost/image.png",
            "custom://example.com/value",
            "http://example.com/insecure",
            "https://user@example.com/private",
            "https://user:password@example.com/private",
            "https:///missing-host.png",
            "https://exa mple.com/image.png",
            "https://example.com/path\nnext",
            "https:\\example.com/image.png",
            "not a URL",
        ] {
            assert!(validate_store_page_url(invalid).is_err(), "{invalid}");
        }
        assert!(validate_store_page_url(&format!(
            "https://example.com/{}",
            "x".repeat(MAX_URL_BYTES)
        ))
        .is_err());
    }

    #[test]
    fn markdown_strips_raw_html_and_scriptable_attributes() {
        let source = concat!(
            "<script>alert(1)</script>\n",
            "<img src=x onerror=alert(1)>\n",
            "<iframe src=\"https://example.com\"></iframe>\n",
            "<object data=\"https://example.com\"></object>\n",
            "<embed src=\"https://example.com\">\n",
            "<span style=\"position:fixed\" onclick=\"alert(1)\">unsafe</span>\n\n",
            "Safe **text**"
        );
        let (sanitized, diagnostics) = sanitize_markdown(
            source,
            MAX_MARKDOWN_SOURCE_BYTES,
            MAX_MARKDOWN_OUTPUT_BYTES,
            "test",
        )
        .expect("sanitized Markdown");
        let output = sanitized.as_str().to_ascii_lowercase();
        for forbidden in [
            "<script", "onerror", "<iframe", "<object", "<embed", "style=", "onclick",
        ] {
            assert!(!output.contains(forbidden), "{forbidden}: {output}");
        }
        assert!(output.contains("safe <strong>text</strong>"));
        assert!(diagnostics.contains(&MarkdownDiagnostic::RawHtmlRemoved));
    }

    #[test]
    fn markdown_removes_unsafe_link_and_image_destinations() {
        let source = concat!(
            "[bad](JaVaScRiPt:alert(1)) ",
            "![tracking](data:image/svg+xml,%3Csvg%3E) ",
            "[good](https://example.com/page) ",
            "![safe](https://example.com/image.webp)"
        );
        let (sanitized, diagnostics) = sanitize_markdown(
            source,
            MAX_MARKDOWN_SOURCE_BYTES,
            MAX_MARKDOWN_OUTPUT_BYTES,
            "test",
        )
        .expect("sanitized Markdown");
        let output = sanitized.as_str();
        assert!(!output.to_ascii_lowercase().contains("javascript:"));
        assert!(!output.to_ascii_lowercase().contains("data:"));
        assert_eq!(output.matches("<a ").count(), 1);
        assert_eq!(output.matches("<img").count(), 1);
        assert!(output.contains("bad"));
        assert!(output.contains("href=\"https://example.com/page\""));
        assert!(output.contains("target=\"_blank\" rel=\"noopener noreferrer\""));
        assert!(output.contains("src=\"https://example.com/image.webp\""));
        assert!(diagnostics.contains(&MarkdownDiagnostic::UnsafeLinkRemoved));
        assert!(diagnostics.contains(&MarkdownDiagnostic::UnsafeImageRemoved));
    }

    #[test]
    fn markdown_rejects_excessive_nesting() {
        let source = format!("{}nested", "> ".repeat(MAX_MARKDOWN_NESTING_DEPTH + 1));
        assert!(matches!(
            sanitize_markdown(
                &source,
                MAX_MARKDOWN_SOURCE_BYTES,
                MAX_MARKDOWN_OUTPUT_BYTES,
                "test"
            ),
            Err(ContentPolicyError::LimitExceeded { .. })
        ));
    }

    #[test]
    fn markdown_rejects_oversized_sanitized_output() {
        let source = "&".repeat(MAX_MARKDOWN_OUTPUT_BYTES / 4);
        assert!(source.len() < MAX_MARKDOWN_SOURCE_BYTES);
        assert!(matches!(
            sanitize_markdown(
                &source,
                MAX_MARKDOWN_SOURCE_BYTES,
                MAX_MARKDOWN_OUTPUT_BYTES,
                "test"
            ),
            Err(ContentPolicyError::LimitExceeded { .. })
        ));
    }

    #[test]
    fn direct_video_policy_allows_only_mp4_and_webm_https_urls() {
        assert!(is_allowed_direct_video_url(
            "https://cdn.example.com/trailer.mp4?token=value"
        ));
        assert!(is_allowed_direct_video_url(
            "https://cdn.example.com/trailer.WEBM"
        ));
        for invalid in [
            "https://video.example.com/watch?v=123",
            "https://cdn.example.com/trailer.m3u8",
            "https://cdn.example.com/trailer.mov",
            "http://cdn.example.com/trailer.mp4",
        ] {
            assert!(!is_allowed_direct_video_url(invalid), "{invalid}");
        }
    }
}
