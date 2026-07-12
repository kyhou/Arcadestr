use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::http_client::{HttpClient, HttpClientError, HttpDownloadOutcome};

#[derive(Default)]
struct MockState {
    responses: HashMap<String, VecDeque<Result<Value, HttpClientError>>>,
    no_redirect_responses: HashMap<String, VecDeque<Result<Value, HttpClientError>>>,
    post_responses: HashMap<String, VecDeque<Result<Value, HttpClientError>>>,
    download_responses: HashMap<String, VecDeque<Result<Vec<u8>, HttpClientError>>>,
    prefix_responses: Vec<(String, VecDeque<Result<Value, HttpClientError>>)>,
    call_counts: HashMap<String, usize>,
    no_redirect_call_counts: HashMap<String, usize>,
    post_call_counts: HashMap<String, usize>,
    requested_urls: Vec<String>,
    post_bodies: HashMap<String, Value>,
    post_headers: HashMap<String, Vec<(String, String)>>,
    download_headers: HashMap<String, Vec<(String, String)>>,
}

/// In-memory HTTP mock for unit tests.
#[derive(Clone, Default)]
pub struct MockHttpClient {
    state: Arc<Mutex<MockState>>,
}

impl MockHttpClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_json_response(self, url: &str, body: Value) -> Self {
        self.push_response(url, Ok(body));
        self
    }

    pub fn with_error_response(self, url: &str, err: HttpClientError) -> Self {
        self.push_response(url, Err(err));
        self
    }

    pub fn with_json_post_response(self, url: &str, body: Value) -> Self {
        self.push_post_response(url, Ok(body));
        self
    }

    pub fn with_post_error_response(self, url: &str, err: HttpClientError) -> Self {
        self.push_post_response(url, Err(err));
        self
    }

    pub fn with_download_response(self, url: &str, body: impl Into<Vec<u8>>) -> Self {
        self.push_download_response(url, Ok(body.into()));
        self
    }

    pub fn with_download_error(self, url: &str, err: HttpClientError) -> Self {
        self.push_download_response(url, Err(err));
        self
    }

    pub fn with_no_redirect_json_response(self, url: &str, body: Value) -> Self {
        self.push_no_redirect_response(url, Ok(body));
        self
    }

    pub fn with_no_redirect_error_response(self, url: &str, err: HttpClientError) -> Self {
        self.push_no_redirect_response(url, Err(err));
        self
    }

    pub fn with_prefix_json_response(self, prefix: &str, body: Value) -> Self {
        self.push_prefix_response(prefix, Ok(body));
        self
    }

    pub fn with_nip05_response(self, domain: &str, name: &str, pubkey_hex: &str) -> Self {
        let url = format!("https://{}/.well-known/nostr.json?name={}", domain, name);
        let body = json!({
            "names": {
                name: pubkey_hex,
            }
        });

        self.with_json_response(&url, body)
    }

    pub fn with_redirect_response(self, url: &str, location: &str) -> Self {
        self.with_error_response(url, HttpClientError::RedirectBlocked(location.to_string()))
    }

    pub fn with_lnurl_response(self, lud16: &str, callback: &str) -> Self {
        let parts: Vec<&str> = lud16.split('@').collect();
        if parts.len() != 2 {
            return self;
        }

        let user = parts[0];
        let domain = parts[1];
        let url = format!("https://{}/.well-known/lnurlp/{}", domain, user);
        let body = json!({
            "callback": callback,
            "minSendable": 1000,
            "maxSendable": 1000000,
            "allowsNostr": true,
            "nostrPubkey": "d94a3f0b5b907fda6c1d2716af34e4d533ddf8f6f6f0f8f1f4a3f605f6c9a3b4"
        });

        self.with_json_response(&url, body)
    }

    pub fn call_count(&self, url: &str) -> usize {
        self.state
            .lock()
            .expect("mock_http_client state mutex poisoned")
            .call_counts
            .get(url)
            .copied()
            .unwrap_or(0)
    }

    pub fn no_redirect_call_count(&self, url: &str) -> usize {
        self.state
            .lock()
            .expect("mock_http_client state mutex poisoned")
            .no_redirect_call_counts
            .get(url)
            .copied()
            .unwrap_or(0)
    }

    pub fn post_call_count(&self, url: &str) -> usize {
        self.state
            .lock()
            .expect("mock_http_client state mutex poisoned")
            .post_call_counts
            .get(url)
            .copied()
            .unwrap_or(0)
    }

    pub fn last_json_post_body(&self, url: &str) -> Option<Value> {
        self.state
            .lock()
            .expect("mock_http_client state mutex poisoned")
            .post_bodies
            .get(url)
            .cloned()
    }

    pub fn last_json_post_headers(&self, url: &str) -> Option<Vec<(String, String)>> {
        self.state
            .lock()
            .expect("mock_http_client state mutex poisoned")
            .post_headers
            .get(url)
            .cloned()
    }

    pub fn last_download_headers(&self, url: &str) -> Option<Vec<(String, String)>> {
        self.state
            .lock()
            .expect("mock_http_client state mutex poisoned")
            .download_headers
            .get(url)
            .cloned()
    }

    pub fn last_requested_url(&self) -> Option<String> {
        self.state
            .lock()
            .expect("mock_http_client state mutex poisoned")
            .requested_urls
            .last()
            .cloned()
    }

    fn push_response(&self, url: &str, response: Result<Value, HttpClientError>) {
        let mut state = self
            .state
            .lock()
            .expect("mock_http_client state mutex poisoned");
        state
            .responses
            .entry(url.to_string())
            .or_default()
            .push_back(response);
    }

    fn push_no_redirect_response(&self, url: &str, response: Result<Value, HttpClientError>) {
        let mut state = self
            .state
            .lock()
            .expect("mock_http_client state mutex poisoned");
        state
            .no_redirect_responses
            .entry(url.to_string())
            .or_default()
            .push_back(response);
    }

    fn push_post_response(&self, url: &str, response: Result<Value, HttpClientError>) {
        let mut state = self
            .state
            .lock()
            .expect("mock_http_client state mutex poisoned");
        state
            .post_responses
            .entry(url.to_string())
            .or_default()
            .push_back(response);
    }

    fn push_download_response(&self, url: &str, response: Result<Vec<u8>, HttpClientError>) {
        let mut state = self
            .state
            .lock()
            .expect("mock_http_client state mutex poisoned");
        state
            .download_responses
            .entry(url.to_string())
            .or_default()
            .push_back(response);
    }

    fn push_prefix_response(&self, prefix: &str, response: Result<Value, HttpClientError>) {
        let mut state = self
            .state
            .lock()
            .expect("mock_http_client state mutex poisoned");

        if let Some((_, queue)) = state
            .prefix_responses
            .iter_mut()
            .find(|(existing_prefix, _)| existing_prefix == prefix)
        {
            queue.push_back(response);
            return;
        }

        let mut queue = VecDeque::new();
        queue.push_back(response);
        state.prefix_responses.push((prefix.to_string(), queue));
    }
}

#[async_trait]
impl HttpClient for MockHttpClient {
    async fn get_json(&self, url: &str) -> Result<Value, HttpClientError> {
        let mut state = self
            .state
            .lock()
            .expect("mock_http_client state mutex poisoned");

        state.requested_urls.push(url.to_string());
        *state.call_counts.entry(url.to_string()).or_insert(0) += 1;

        match state.responses.get_mut(url) {
            Some(queue) if !queue.is_empty() => pop_response(queue),
            _ => {
                for (prefix, queue) in &mut state.prefix_responses {
                    if url.starts_with(prefix.as_str()) && !queue.is_empty() {
                        return pop_response(queue);
                    }
                }

                Err(HttpClientError::Request(format!(
                    "No mock response configured for URL: {}",
                    url
                )))
            }
        }
    }

    async fn get_json_no_redirects(&self, url: &str) -> Result<Value, HttpClientError> {
        let mut state = self
            .state
            .lock()
            .expect("mock_http_client state mutex poisoned");

        state.requested_urls.push(url.to_string());
        *state.call_counts.entry(url.to_string()).or_insert(0) += 1;
        *state
            .no_redirect_call_counts
            .entry(url.to_string())
            .or_insert(0) += 1;

        match state.no_redirect_responses.get_mut(url) {
            Some(queue) if !queue.is_empty() => pop_response(queue),
            _ => match state.responses.get_mut(url) {
                Some(queue) if !queue.is_empty() => pop_response(queue),
                _ => {
                    for (prefix, queue) in &mut state.prefix_responses {
                        if url.starts_with(prefix.as_str()) && !queue.is_empty() {
                            return pop_response(queue);
                        }
                    }

                    Err(HttpClientError::Request(format!(
                        "No mock response configured for URL: {}",
                        url
                    )))
                }
            },
        }
    }

    async fn post_json(
        &self,
        url: &str,
        body: Value,
        headers: Vec<(String, String)>,
    ) -> Result<Value, HttpClientError> {
        let mut state = self
            .state
            .lock()
            .expect("mock_http_client state mutex poisoned");

        state.requested_urls.push(url.to_string());
        *state.call_counts.entry(url.to_string()).or_insert(0) += 1;
        *state.post_call_counts.entry(url.to_string()).or_insert(0) += 1;
        state.post_bodies.insert(url.to_string(), body);
        state.post_headers.insert(url.to_string(), headers);

        match state.post_responses.get_mut(url) {
            Some(queue) if !queue.is_empty() => pop_response(queue),
            _ => Err(HttpClientError::Request(format!(
                "No mock POST response configured for URL: {}",
                url
            ))),
        }
    }

    async fn download_to_path(
        &self,
        url: &str,
        headers: Vec<(String, String)>,
        dest: &Path,
        on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<HttpDownloadOutcome, HttpClientError> {
        let response = {
            let mut state = self
                .state
                .lock()
                .expect("mock_http_client state mutex poisoned");

            state.requested_urls.push(url.to_string());
            *state.call_counts.entry(url.to_string()).or_insert(0) += 1;
            state.download_headers.insert(url.to_string(), headers);

            match state.download_responses.get_mut(url) {
                Some(queue) if !queue.is_empty() => queue.pop_front(),
                _ => None,
            }
        };

        let bytes = match response {
            Some(Ok(bytes)) => bytes,
            Some(Err(err)) => return Err(err),
            None => {
                return Err(HttpClientError::Request(format!(
                    "No mock download response configured for URL: {}",
                    url
                )))
            }
        };

        std::fs::write(dest, &bytes).map_err(|e| HttpClientError::Request(e.to_string()))?;
        let bytes_written = bytes.len() as u64;
        on_progress(bytes_written, Some(bytes_written));

        Ok(HttpDownloadOutcome { bytes_written })
    }
}

fn pop_response(
    queue: &mut VecDeque<Result<Value, HttpClientError>>,
) -> Result<Value, HttpClientError> {
    if queue.len() == 1 {
        return queue[0].clone();
    }

    queue
        .pop_front()
        .expect("mock response queue unexpectedly empty")
}
