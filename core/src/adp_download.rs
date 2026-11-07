//! Authorized, artifact-pinned failover over the existing ADP/HTTP verifier.
use crate::{
    adp_client::{AdpClient, AdpClientError, DownloadAuth, RetainedDownloadOutcome},
    distribution::{PackageFormat, Sha256Digest},
    http_client::{HttpClient, HttpClientError},
    package_extraction::{ExtractionError, VerifiedPackage},
    signers::NostrSigner,
};
use std::{
    fs::File,
    io::{Seek, SeekFrom},
    sync::Arc,
};

pub struct DownloadCandidate {
    pub server_url: String,
    pub token: Option<String>,
}

/// Preserve established preference: valid tokens in signed server order, then
/// other signed servers with independent NIP-98 eligibility. Tokens never move.
pub fn download_candidates(
    servers: &[String],
    tokens: &[(String, String)],
    nip98_eligible: bool,
) -> Vec<DownloadCandidate> {
    let mut selected = std::collections::HashSet::new();
    let mut result = Vec::new();
    for server in servers {
        if let Some((_, token)) = tokens
            .iter()
            .find(|(s, _)| s.trim_end_matches('/') == server.trim_end_matches('/'))
        {
            if selected.insert(server.trim_end_matches('/').to_string()) {
                result.push(DownloadCandidate {
                    server_url: server.clone(),
                    token: Some(token.clone()),
                });
            }
        }
    }
    if nip98_eligible {
        for server in servers {
            if selected.insert(server.trim_end_matches('/').to_string()) {
                result.push(DownloadCandidate {
                    server_url: server.clone(),
                    token: None,
                });
            }
        }
    }
    result
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadFailure {
    #[error(transparent)]
    Adp(#[from] AdpClientError),
    #[error(transparent)]
    Package(#[from] ExtractionError),
}
pub struct VerifiedDownload {
    pub download: RetainedDownloadOutcome,
    pub package: VerifiedPackage,
}

fn retryable(error: &AdpClientError) -> bool {
    match error {
        AdpClientError::DownloadUnavailable(_) => true,
        AdpClientError::Http(HttpClientError::Request(_)) => true,
        AdpClientError::DownloadProtocol(message) => [
            "HTTP status 404",
            "HTTP status 408",
            "HTTP status 429",
            "HTTP status 500",
            "HTTP status 502",
            "HTTP status 504",
        ]
        .iter()
        .any(|s| message.starts_with(s)),
        _ => false,
    }
}
fn safe_error(error: AdpClientError) -> AdpClientError {
    match error {
        AdpClientError::DownloadOwnership(_) => AdpClientError::DownloadOwnership(
            "Ownership check failed; download stopped without trying other credentials".into(),
        ),
        AdpClientError::DownloadDistribution(_) => AdpClientError::DownloadDistribution(
            "Server distribution authorization failed; refresh the signed listing before retrying"
                .into(),
        ),
        AdpClientError::Http(HttpClientError::InsufficientStorage(message)) => {
            AdpClientError::Http(HttpClientError::InsufficientStorage(message))
        }
        AdpClientError::Http(HttpClientError::FilesystemFileSizeLimit(message)) => {
            AdpClientError::Http(HttpClientError::FilesystemFileSizeLimit(message))
        }
        AdpClientError::Http(HttpClientError::DownloadIo(_)) => AdpClientError::Http(
            HttpClientError::DownloadIo("could not write the private download candidate".into()),
        ),
        AdpClientError::Http(HttpClientError::Status(413))
        | AdpClientError::Http(HttpClientError::StatusWithBody { status: 413, .. }) => {
            AdpClientError::Http(HttpClientError::Status(413))
        }
        AdpClientError::DownloadProtocol(message) if message.starts_with("HTTP status 401") => {
            AdpClientError::DownloadProtocol(
                "HTTP status 401: download authentication rejected".into(),
            )
        }
        _ => AdpClientError::DownloadUnavailable(
            "Authorized download source unavailable; credentials and response bodies omitted"
                .into(),
        ),
    }
}

/// Each attempt starts at byte zero on the same private capability-owned file.
/// Verification pins the same digest/format for every server. No redirects,
/// permission downgrades or alternative-artifact selection are introduced.
pub async fn download_verified(
    http: Arc<dyn HttpClient>,
    coordinate: &str,
    buyer: &str,
    signer: &dyn NostrSigner,
    candidates: Vec<DownloadCandidate>,
    mut file: File,
    digest: Sha256Digest,
    format: PackageFormat,
    mut preflight: impl FnMut(Option<u64>) -> Result<(), HttpClientError> + Send,
    mut progress: impl FnMut(u64, Option<u64>) + Send,
    mut verifying: impl FnMut(u64) + Send,
) -> Result<VerifiedDownload, DownloadFailure> {
    let mut last = DownloadFailure::Adp(AdpClientError::DownloadUnavailable(
        "No eligible authorized download source".into(),
    ));
    for candidate in candidates {
        if signer
            .get_public_key()
            .await
            .map_err(|_| {
                AdpClientError::DownloadProtocol("Active download signer unavailable".into())
            })?
            .to_hex()
            != buyer
        {
            return Err(
                AdpClientError::DownloadProtocol("Active download account changed".into()).into(),
            );
        }
        file.set_len(0)
            .and_then(|_| file.seek(SeekFrom::Start(0)))
            .map_err(|_| {
                AdpClientError::DownloadProtocol(
                    "Private download candidate could not be reset".into(),
                )
            })?;
        let auth = match candidate.token {
            Some(token) => DownloadAuth::Token(token),
            None => DownloadAuth::Nip98 { signer },
        };
        progress(0, None);
        let result = AdpClient::new(candidate.server_url, http.clone())
            .download_retained_to_open_file_with_preflight(
                coordinate,
                auth,
                file.try_clone().map_err(|_| {
                    AdpClientError::DownloadProtocol("Private download handle unavailable".into())
                })?,
                &mut preflight,
                &mut progress,
            )
            .await;
        let download = match result {
            Ok(download) => download,
            Err(error) => {
                let retry = retryable(&error);
                last = DownloadFailure::Adp(safe_error(error));
                if retry {
                    continue;
                } else {
                    return Err(last);
                }
            }
        };
        verifying(download.bytes_written);
        let handle = download.file.clone();
        let expected = digest.clone();
        let package_format = format.clone();
        let verified = tokio::task::spawn_blocking(move || {
            VerifiedPackage::verify(handle, expected, package_format)
        })
        .await
        .map_err(|_| AdpClientError::DownloadProtocol("Package verification task failed".into()))?;
        match verified {
            Ok(package) => return Ok(VerifiedDownload { download, package }),
            Err(error @ ExtractionError::DigestMismatch { .. }) => {
                last = DownloadFailure::Package(error)
            }
            Err(error) => return Err(DownloadFailure::Package(error)),
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidates_preserve_signed_order_tokens_and_local_eligibility() {
        let servers = vec![
            "https://one.example".into(),
            "https://two.example/".into(),
            "https://three.example".into(),
            "https://two.example".into(),
        ];
        let tokens = vec![
            ("https://unauthorized.example".into(), "never-send".into()),
            ("https://two.example/".into(), "two-only".into()),
        ];
        let candidates = download_candidates(&servers, &tokens, true);
        assert_eq!(
            candidates
                .iter()
                .map(|c| c.server_url.as_str())
                .collect::<Vec<_>>(),
            vec![
                "https://two.example/",
                "https://one.example",
                "https://three.example"
            ]
        );
        assert_eq!(candidates[0].token.as_deref(), Some("two-only"));
        assert!(candidates[1..].iter().all(|c| c.token.is_none()));
        assert_eq!(download_candidates(&servers, &tokens, false).len(), 1);
        assert!(download_candidates(&servers, &[], false).is_empty());
    }
    #[test]
    fn entitlement_auth_and_local_failures_are_never_retryable() {
        for error in [
            AdpClientError::DownloadOwnership("denied".into()),
            AdpClientError::DownloadDistribution("denied".into()),
            AdpClientError::DownloadProtocol("HTTP status 401".into()),
            AdpClientError::Http(HttpClientError::InsufficientStorage("full".into())),
        ] {
            assert!(!retryable(&error));
        }
    }
}
