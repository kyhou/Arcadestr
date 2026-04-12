//! HTTP client abstraction for testable network-dependent modules.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

/// Errors returned by [`HttpClient`] implementations.
#[derive(Debug, Clone, Error)]
pub enum HttpClientError {
    #[error("HTTP client build failed: {0}")]
    Build(String),
    #[error("HTTP request failed: {0}")]
    Request(String),
    #[error("HTTP redirect blocked: {0}")]
    RedirectBlocked(String),
    #[error("HTTP status not successful: {0}")]
    Status(u16),
    #[error("HTTP JSON decode failed: {0}")]
    Json(String),
}

/// Minimal HTTP client contract used by protocol validators.
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// Fetch URL and decode JSON payload.
    async fn get_json(&self, url: &str) -> Result<Value, HttpClientError>;
}

/// Production HTTP client backed by `reqwest`.
pub struct ReqwestHttpClient {
    client: reqwest::Client,
}

impl ReqwestHttpClient {
    /// Build client with redirect policy disabled for NIP-05 compliance.
    pub fn new(timeout: Duration) -> Result<Self, HttpClientError> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| HttpClientError::Build(e.to_string()))?;

        Ok(Self { client })
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn get_json(&self, url: &str) -> Result<Value, HttpClientError> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| HttpClientError::Request(e.to_string()))?;

        if response.status().is_redirection() {
            return Err(HttpClientError::RedirectBlocked(
                response.status().to_string(),
            ));
        }

        if !response.status().is_success() {
            return Err(HttpClientError::Status(response.status().as_u16()));
        }

        response
            .json::<Value>()
            .await
            .map_err(|e| HttpClientError::Json(e.to_string()))
    }
}
