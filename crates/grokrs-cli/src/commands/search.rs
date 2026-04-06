//! Shared search configuration and citation formatting for `grokrs chat`
//! and `grokrs agent` commands.
//!
//! Both the chat REPL and agent command support server-side search tools
//! (`web_search`, `x_search`). This module provides the shared types and
//! logic for building search tool arrays, constructing `SearchParameters`,
//! and formatting citations returned in API responses.

use grokrs_api::types::builtin_tools::{BuiltinTool, SearchMode, SearchParameters};

// ---------------------------------------------------------------------------
// SearchConfig — resolved search settings from CLI flags
// ---------------------------------------------------------------------------

/// Resolved search configuration built from CLI flags.
///
/// Both `ChatArgs` and `AgentArgs` contribute flags that are resolved into
/// this structure. If no search flags are set, `is_empty()` returns `true`
/// and neither tools nor parameters should be added to the request.
#[derive(Debug, Clone, Default)]
pub struct SearchConfig {
    /// Include `BuiltinTool::WebSearch` in the tools array.
    pub web_search: bool,
    /// Include `BuiltinTool::XSearch` in the tools array.
    pub x_search: bool,
    /// Earliest date for search results (ISO 8601, e.g. `"2025-01-01"`).
    pub from_date: Option<String>,
    /// Latest date for search results (ISO 8601, e.g. `"2025-12-31"`).
    pub to_date: Option<String>,
    /// Maximum number of search results to return.
    pub max_results: Option<u32>,
    /// Whether to request citations in the response.
    pub citations: bool,
}

impl SearchConfig {
    /// Returns `true` if no search tools are enabled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.web_search && !self.x_search
    }

    /// Build the list of `BuiltinTool` values to include in the request tools array.
    ///
    /// Returns an empty vec if no search tools are enabled.
    #[must_use]
    pub fn builtin_tools(&self) -> Vec<BuiltinTool> {
        let mut tools = Vec::new();
        if self.web_search {
            tools.push(BuiltinTool::WebSearch);
        }
        if self.x_search {
            tools.push(BuiltinTool::XSearch);
        }
        tools
    }

    /// Build `serde_json::Value` representations of the search tools,
    /// suitable for appending to the request's `tools` array.
    #[must_use]
    pub fn tool_values(&self) -> Vec<serde_json::Value> {
        self.builtin_tools()
            .iter()
            .map(BuiltinTool::to_value)
            .collect()
    }

    /// Build `SearchParameters` from this config, or `None` if no parameters
    /// are needed (no date range, no max results, no citations).
    #[must_use]
    pub fn search_parameters(&self) -> Option<SearchParameters> {
        if self.is_empty() {
            return None;
        }

        let has_params = self.from_date.is_some()
            || self.to_date.is_some()
            || self.max_results.is_some()
            || self.citations;

        if !has_params {
            return None;
        }

        Some(SearchParameters {
            mode: Some(SearchMode::Auto),
            sources: None,
            from_date: self.from_date.clone(),
            to_date: self.to_date.clone(),
            max_search_results: self.max_results,
            return_citations: if self.citations { Some(true) } else { None },
        })
    }
}

// ---------------------------------------------------------------------------
// Citation formatting
// ---------------------------------------------------------------------------

/// A single citation extracted from an API response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation {
    /// The citation URL.
    pub url: String,
    /// Optional title or label for the citation.
    pub title: Option<String>,
}

/// Extract citations from a `ResponseCompleted` event's response JSON.
///
/// The Responses API may include citations in the response output items
/// as part of `web_search_call` or `x_search_call` results, or in a
/// top-level `citations` array. This function searches both locations.
#[must_use]
pub fn extract_citations(response: &serde_json::Value) -> Vec<Citation> {
    let mut citations = Vec::new();

    // Check top-level "citations" array (used in some response formats).
    if let Some(arr) = response.get("citations").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
                citations.push(Citation {
                    url: url.to_owned(),
                    title: item
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(ToOwned::to_owned),
                });
            }
        }
    }

    // Check output items for search_results with URLs.
    if let Some(output) = response.get("output").and_then(|v| v.as_array()) {
        for item in output {
            let search_results = item
                .get("search_results")
                .and_then(|v| v.as_array())
                .or_else(|| item.get("results").and_then(|v| v.as_array()));

            if let Some(results) = search_results {
                for result in results {
                    if let Some(url) = result.get("url").and_then(|v| v.as_str()) {
                        let title = result
                            .get("title")
                            .and_then(|v| v.as_str())
                            .map(ToOwned::to_owned);
                        // Avoid duplicates.
                        let citation = Citation {
                            url: url.to_owned(),
                            title,
                        };
                        if !citations.contains(&citation) {
                            citations.push(citation);
                        }
                    }
                }
            }
        }
    }

    citations
}

/// Format citations as numbered references for display.
///
/// Returns an empty string if no citations are provided.
///
/// # Example output
///
/// ```text
/// Sources:
///   [1] Example Article — https://example.com/article
///   [2] https://other.com/page
/// ```
#[must_use]
pub fn format_citations(citations: &[Citation]) -> String {
    if citations.is_empty() {
        return String::new();
    }

    let mut out = String::from("\nSources:\n");
    for (i, cite) in citations.iter().enumerate() {
        match &cite.title {
            Some(title) => {
                out.push_str(&format!("  [{}] {} — {}\n", i + 1, title, cite.url));
            }
            None => {
                out.push_str(&format!("  [{}] {}\n", i + 1, cite.url));
            }
        }
    }
    out
}

/// Extract citations from the output items of a `ResponseObject`.
///
/// Used by the agent command to extract citations from the final response
/// after the tool loop completes.
#[must_use]
pub fn extract_citations_from_output(
    output: &[grokrs_api::types::responses::OutputItem],
) -> Vec<Citation> {
    use grokrs_api::types::responses::OutputItem;

    let mut citations = Vec::new();

    for item in output {
        let search_results = match item {
            OutputItem::WebSearchCall { search_results, .. } => search_results.as_ref(),
            OutputItem::XSearchCall { search_results, .. } => search_results.as_ref(),
            _ => None,
        };

        if let Some(results) = search_results {
            for result in results {
                if let Some(url) = result.get("url").and_then(|v| v.as_str()) {
                    let title = result
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(ToOwned::to_owned);
                    let citation = Citation {
                        url: url.to_owned(),
                        title,
                    };
                    if !citations.contains(&citation) {
                        citations.push(citation);
                    }
                }
            }
        }
    }

    citations
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate an ISO 8601 date string (YYYY-MM-DD format).
///
/// Returns `Ok(())` if the date matches the expected format, or an error
/// message suitable for display to the user.
pub fn validate_date(date: &str) -> Result<(), String> {
    if date.len() != 10 {
        return Err(format!(
            "invalid date format: '{date}' (expected YYYY-MM-DD)"
        ));
    }
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return Err(format!(
            "invalid date format: '{date}' (expected YYYY-MM-DD)"
        ));
    }
    // Validate each part is numeric with correct lengths.
    if parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return Err(format!(
            "invalid date format: '{date}' (expected YYYY-MM-DD)"
        ));
    }
    for part in &parts {
        if part.parse::<u32>().is_err() {
            return Err(format!(
                "invalid date format: '{date}' (expected YYYY-MM-DD)"
            ));
        }
    }
    // Basic range checks.
    let month: u32 = parts[1].parse().unwrap();
    let day: u32 = parts[2].parse().unwrap();
    if !(1..=12).contains(&month) {
        return Err(format!("invalid month in date: '{date}'"));
    }
    if !(1..=31).contains(&day) {
        return Err(format!("invalid day in date: '{date}'"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- SearchConfig --

    #[test]
    fn search_config_default_is_empty() {
        let config = SearchConfig::default();
        assert!(config.is_empty());
        assert!(config.builtin_tools().is_empty());
        assert!(config.tool_values().is_empty());
        assert!(config.search_parameters().is_none());
    }

    #[test]
    fn search_config_web_search_only() {
        let config = SearchConfig {
            web_search: true,
            ..Default::default()
        };
        assert!(!config.is_empty());
        let tools = config.builtin_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0], BuiltinTool::WebSearch);
    }

    #[test]
    fn search_config_x_search_only() {
        let config = SearchConfig {
            x_search: true,
            ..Default::default()
        };
        assert!(!config.is_empty());
        let tools = config.builtin_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0], BuiltinTool::XSearch);
    }

    #[test]
    fn search_config_both_search_tools() {
        let config = SearchConfig {
            web_search: true,
            x_search: true,
            ..Default::default()
        };
        let tools = config.builtin_tools();
        assert_eq!(tools.len(), 2);
        assert!(tools.contains(&BuiltinTool::WebSearch));
        assert!(tools.contains(&BuiltinTool::XSearch));
    }

    #[test]
    fn search_config_tool_values_serialize_correctly() {
        let config = SearchConfig {
            web_search: true,
            x_search: true,
            ..Default::default()
        };
        let values = config.tool_values();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["type"], "web_search");
        assert_eq!(values[1]["type"], "x_search");
    }

    #[test]
    fn search_parameters_none_when_no_params() {
        let config = SearchConfig {
            web_search: true,
            ..Default::default()
        };
        // Search enabled but no parameters -> no SearchParameters.
        assert!(config.search_parameters().is_none());
    }

    #[test]
    fn search_parameters_none_when_search_disabled() {
        let config = SearchConfig {
            from_date: Some("2025-01-01".into()),
            ..Default::default()
        };
        // Parameters set but no search tools -> no SearchParameters.
        assert!(config.search_parameters().is_none());
    }

    #[test]
    fn search_parameters_with_date_range() {
        let config = SearchConfig {
            web_search: true,
            from_date: Some("2025-01-01".into()),
            to_date: Some("2025-12-31".into()),
            ..Default::default()
        };
        let params = config.search_parameters().unwrap();
        assert_eq!(params.from_date.as_deref(), Some("2025-01-01"));
        assert_eq!(params.to_date.as_deref(), Some("2025-12-31"));
        assert_eq!(params.mode, Some(SearchMode::Auto));
    }

    #[test]
    fn search_parameters_with_max_results() {
        let config = SearchConfig {
            web_search: true,
            max_results: Some(5),
            ..Default::default()
        };
        let params = config.search_parameters().unwrap();
        assert_eq!(params.max_search_results, Some(5));
    }

    #[test]
    fn search_parameters_with_citations() {
        let config = SearchConfig {
            web_search: true,
            citations: true,
            ..Default::default()
        };
        let params = config.search_parameters().unwrap();
        assert_eq!(params.return_citations, Some(true));
    }

    #[test]
    fn search_parameters_full() {
        let config = SearchConfig {
            web_search: true,
            x_search: true,
            from_date: Some("2025-01-01".into()),
            to_date: Some("2025-06-30".into()),
            max_results: Some(10),
            citations: true,
        };
        let params = config.search_parameters().unwrap();
        assert_eq!(params.mode, Some(SearchMode::Auto));
        assert_eq!(params.from_date.as_deref(), Some("2025-01-01"));
        assert_eq!(params.to_date.as_deref(), Some("2025-06-30"));
        assert_eq!(params.max_search_results, Some(10));
        assert_eq!(params.return_citations, Some(true));
    }

    // -- Citation extraction --

    #[test]
    fn extract_citations_from_top_level() {
        let response = serde_json::json!({
            "id": "resp_1",
            "citations": [
                {"url": "https://example.com", "title": "Example"},
                {"url": "https://other.com"}
            ]
        });
        let citations = extract_citations(&response);
        assert_eq!(citations.len(), 2);
        assert_eq!(citations[0].url, "https://example.com");
        assert_eq!(citations[0].title.as_deref(), Some("Example"));
        assert_eq!(citations[1].url, "https://other.com");
        assert!(citations[1].title.is_none());
    }

    #[test]
    fn extract_citations_from_search_results() {
        let response = serde_json::json!({
            "id": "resp_1",
            "output": [
                {
                    "type": "web_search_call",
                    "id": "ws_1",
                    "status": "completed",
                    "search_results": [
                        {"url": "https://result1.com", "title": "Result 1"},
                        {"url": "https://result2.com", "title": "Result 2"}
                    ]
                }
            ]
        });
        let citations = extract_citations(&response);
        assert_eq!(citations.len(), 2);
        assert_eq!(citations[0].url, "https://result1.com");
        assert_eq!(citations[1].url, "https://result2.com");
    }

    #[test]
    fn extract_citations_deduplicates() {
        let response = serde_json::json!({
            "id": "resp_1",
            "citations": [
                {"url": "https://example.com", "title": "Example"}
            ],
            "output": [
                {
                    "type": "web_search_call",
                    "id": "ws_1",
                    "search_results": [
                        {"url": "https://example.com", "title": "Example"}
                    ]
                }
            ]
        });
        let citations = extract_citations(&response);
        assert_eq!(citations.len(), 1);
    }

    #[test]
    fn extract_citations_empty_response() {
        let response = serde_json::json!({"id": "resp_1"});
        let citations = extract_citations(&response);
        assert!(citations.is_empty());
    }

    #[test]
    fn extract_citations_from_output_items() {
        use grokrs_api::types::responses::OutputItem;

        let output = vec![
            OutputItem::WebSearchCall {
                id: "ws_1".into(),
                status: Some("completed".into()),
                search_results: Some(vec![
                    serde_json::json!({"url": "https://a.com", "title": "A"}),
                    serde_json::json!({"url": "https://b.com"}),
                ]),
            },
            OutputItem::XSearchCall {
                id: "xs_1".into(),
                status: Some("completed".into()),
                search_results: Some(vec![
                    serde_json::json!({"url": "https://x.com/post", "title": "Post"}),
                ]),
            },
            OutputItem::Message {
                role: grokrs_api::types::common::Role::Assistant,
                content: vec![],
            },
        ];
        let citations = extract_citations_from_output(&output);
        assert_eq!(citations.len(), 3);
        assert_eq!(citations[0].url, "https://a.com");
        assert_eq!(citations[1].url, "https://b.com");
        assert_eq!(citations[2].url, "https://x.com/post");
    }

    #[test]
    fn extract_citations_from_output_empty() {
        let citations = extract_citations_from_output(&[]);
        assert!(citations.is_empty());
    }

    // -- Citation formatting --

    #[test]
    fn format_citations_empty() {
        assert!(format_citations(&[]).is_empty());
    }

    #[test]
    fn format_citations_with_title() {
        let citations = vec![Citation {
            url: "https://example.com".into(),
            title: Some("Example".into()),
        }];
        let formatted = format_citations(&citations);
        assert!(formatted.contains("Sources:"));
        assert!(formatted.contains("[1] Example — https://example.com"));
    }

    #[test]
    fn format_citations_without_title() {
        let citations = vec![Citation {
            url: "https://example.com".into(),
            title: None,
        }];
        let formatted = format_citations(&citations);
        assert!(formatted.contains("[1] https://example.com"));
        assert!(!formatted.contains(" — "));
    }

    #[test]
    fn format_citations_multiple() {
        let citations = vec![
            Citation {
                url: "https://a.com".into(),
                title: Some("A".into()),
            },
            Citation {
                url: "https://b.com".into(),
                title: None,
            },
            Citation {
                url: "https://c.com".into(),
                title: Some("C Article".into()),
            },
        ];
        let formatted = format_citations(&citations);
        assert!(formatted.contains("[1] A — https://a.com"));
        assert!(formatted.contains("[2] https://b.com"));
        assert!(formatted.contains("[3] C Article — https://c.com"));
    }

    // -- Date validation --

    #[test]
    fn validate_date_valid() {
        assert!(validate_date("2025-01-01").is_ok());
        assert!(validate_date("2025-12-31").is_ok());
        assert!(validate_date("2000-06-15").is_ok());
    }

    #[test]
    fn validate_date_invalid_format() {
        assert!(validate_date("2025/01/01").is_err());
        assert!(validate_date("25-01-01").is_err());
        assert!(validate_date("not-a-date").is_err());
        assert!(validate_date("").is_err());
    }

    #[test]
    fn validate_date_invalid_month() {
        assert!(validate_date("2025-13-01").is_err());
        assert!(validate_date("2025-00-01").is_err());
    }

    #[test]
    fn validate_date_invalid_day() {
        assert!(validate_date("2025-01-00").is_err());
        assert!(validate_date("2025-01-32").is_err());
    }
}
