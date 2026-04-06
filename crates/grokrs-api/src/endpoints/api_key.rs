use reqwest::Method;

use crate::transport::client::HttpClient;
use crate::transport::error::TransportError;
use crate::types::api_key::ApiKeyInfo;

/// Client for the xAI API Key information endpoint.
///
/// Provides access to `GET /v1/api-key` for retrieving metadata about the
/// currently authenticated API key.
///
/// **Important**: API key information must NOT be cached. Always call `info()`
/// to get the current state of the key (status, ACLs, blocked/disabled).
pub struct ApiKeyClient<'a> {
    http: &'a HttpClient,
}

impl<'a> ApiKeyClient<'a> {
    /// Create a new `ApiKeyClient` wrapping the given HTTP client.
    #[must_use]
    pub fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Retrieve information about the currently authenticated API key.
    ///
    /// Sends a `GET /v1/api-key` request and returns the key's metadata
    /// including name, status, ACLs, and blocked/disabled flags.
    ///
    /// This method always makes a fresh request — results are never cached.
    pub async fn info(&self) -> Result<ApiKeyInfo, TransportError> {
        self.http.send_no_body(Method::GET, "/v1/api-key").await
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn api_key_endpoint_path() {
        let path = "/v1/api-key";
        assert_eq!(path, "/v1/api-key");
    }
}
