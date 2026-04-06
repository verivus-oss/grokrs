//! Optional function calling loop helper.
//!
//! Drives a multi-turn function calling conversation with the xAI Responses API.
//! The caller provides a `FunctionExecutor` implementation that handles the
//! actual function execution; this module handles the wire-format plumbing of
//! sending requests, detecting function calls in the output, collecting results,
//! and sending follow-up requests.
//!
//! This is a **convenience** — callers can always drive the loop manually using
//! `ResponsesClient::create()` and inspecting `OutputItem::FunctionCall` variants.

use std::fmt;

use crate::endpoints::responses::ResponsesClient;
use crate::transport::error::TransportError;
use crate::types::responses::{CreateResponseRequest, OutputItem, ResponseObject};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the tool calling loop.
#[derive(Debug, Clone)]
pub struct ToolLoopConfig {
    /// Maximum number of request-response iterations before aborting.
    ///
    /// Each iteration sends a request and processes any function calls in the
    /// response. Defaults to 10.
    pub max_iterations: u32,
}

impl Default for ToolLoopConfig {
    fn default() -> Self {
        Self { max_iterations: 10 }
    }
}

// ---------------------------------------------------------------------------
// FunctionExecutor trait
// ---------------------------------------------------------------------------

/// Trait for executing function calls returned by the model.
///
/// Implementations receive the function name and arguments (as a JSON string)
/// and must return the result as a string (typically JSON).
///
/// # Examples
///
/// ```
/// use grokrs_api::tool_loop::FunctionExecutor;
///
/// struct MyExecutor;
///
/// impl FunctionExecutor for MyExecutor {
///     fn execute(
///         &self,
///         name: &str,
///         arguments: &str,
///     ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
///         match name {
///             "get_weather" => Ok(r#"{"temp": 72, "unit": "F"}"#.into()),
///             _ => Err(format!("unknown function: {name}").into()),
///         }
///     }
/// }
/// ```
pub trait FunctionExecutor: Send + Sync {
    /// Execute a function call and return the result as a string.
    ///
    /// # Arguments
    /// * `name` - The name of the function to execute.
    /// * `arguments` - The arguments as a JSON string.
    ///
    /// # Errors
    /// Returns an error if the function execution fails.
    fn execute(
        &self,
        name: &str,
        arguments: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
}

// ---------------------------------------------------------------------------
// ToolLoopError
// ---------------------------------------------------------------------------

/// Errors that can occur during the tool calling loop.
#[derive(Debug)]
pub enum ToolLoopError {
    /// The loop exceeded the maximum number of iterations without the model
    /// producing a final response without function calls.
    MaxIterationsExceeded {
        /// The number of iterations that were executed.
        iterations: u32,
        /// The configured maximum.
        max: u32,
    },

    /// A function executor returned an error for a specific function call.
    ExecutionFailed {
        /// The name of the function that failed.
        name: String,
        /// The error from the executor.
        error: Box<dyn std::error::Error + Send + Sync>,
    },

    /// An HTTP/API transport error occurred.
    Transport(TransportError),

    /// The caller set `previous_response_id` but did not set `store = true`.
    /// Stateful conversation chaining requires `store = true` so the server
    /// retains the response for continuation.
    InvalidConfiguration {
        /// Description of the misconfiguration.
        message: String,
    },
}

impl fmt::Display for ToolLoopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolLoopError::MaxIterationsExceeded { iterations, max } => {
                write!(
                    f,
                    "tool loop exceeded maximum iterations: {iterations}/{max}"
                )
            }
            ToolLoopError::ExecutionFailed { name, error } => {
                write!(f, "function execution failed for '{name}': {error}")
            }
            ToolLoopError::Transport(err) => {
                write!(f, "transport error in tool loop: {err}")
            }
            ToolLoopError::InvalidConfiguration { message } => {
                write!(f, "invalid tool loop configuration: {message}")
            }
        }
    }
}

impl std::error::Error for ToolLoopError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ToolLoopError::Transport(err) => Some(err),
            ToolLoopError::ExecutionFailed { error, .. } => Some(error.as_ref()),
            _ => None,
        }
    }
}

impl From<TransportError> for ToolLoopError {
    fn from(err: TransportError) -> Self {
        ToolLoopError::Transport(err)
    }
}

// ---------------------------------------------------------------------------
// Extracted function call data
// ---------------------------------------------------------------------------

/// A function call extracted from the response output.
struct PendingCall {
    call_id: String,
    name: String,
    arguments: String,
}

/// Extract all `FunctionCall` items from the response output.
///
/// Only client-side `FunctionCall` items are extracted. Server-side tool calls
/// (including `McpCall`, `WebSearchCall`, `XSearchCall`, `CodeInterpreterCall`,
/// `FileSearchCall`) are handled entirely by the xAI server and are NOT
/// extracted for local execution. The catch-all `_ => None` arm ensures that
/// any future server-side output item types are also safely ignored.
fn extract_function_calls(response: &ResponseObject) -> Vec<PendingCall> {
    response
        .output
        .iter()
        .filter_map(|item| match item {
            OutputItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => Some(PendingCall {
                call_id: call_id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            }),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// run_tool_loop
// ---------------------------------------------------------------------------

/// Drive a function calling loop using the Responses API.
///
/// Sends the `initial_request` to the API, checks the response for
/// `FunctionCall` output items, executes each one via the `executor`, builds
/// `function_call_output` items, and sends a follow-up request.
///
/// The loop operates in one of two modes based on the `store` field of the
/// initial request:
///
/// **Mode A — Stateful chaining (`store = true`):**
/// The server stores each response.  Follow-up requests set
/// `previous_response_id` to the most recent response's `id` and include
/// only the new tool-result items in `input`.  This preserves the full
/// server-side conversation history, including any earlier turns referenced
/// by the caller's `previous_response_id`.
///
/// **Mode B — Stateless rebuild (`store = false` or `None`):**
/// Each follow-up rebuilds the full conversation context in `input` from
/// scratch (original input + accumulated assistant outputs + tool results).
/// No `previous_response_id` is set on follow-ups.
///
/// Repeats until either:
///
/// - The model produces a response with no function calls (success).
/// - The loop exceeds `config.max_iterations` (error).
/// - A function execution or transport error occurs (error).
///
/// # Arguments
///
/// * `client` - The `ResponsesClient` to use for API requests.
/// * `initial_request` - The first request to send (should include tool definitions).
/// * `executor` - An implementation of `FunctionExecutor` that handles function calls.
/// * `config` - Configuration for the loop (max iterations, etc.).
///
/// # Returns
///
/// The final `ResponseObject` whose output contains no more function calls.
///
/// # Errors
///
/// Returns `ToolLoopError` if the loop exceeds max iterations, a function
/// execution fails, or a transport error occurs.
pub async fn run_tool_loop(
    client: &ResponsesClient,
    initial_request: CreateResponseRequest,
    executor: &dyn FunctionExecutor,
    config: ToolLoopConfig,
) -> Result<ResponseObject, ToolLoopError> {
    // Serialize the initial request once so follow-ups inherit ALL caller-set
    // fields (temperature, reasoning, max_output_tokens, metadata, etc.)
    // without needing to enumerate each one.
    let initial_json = serde_json::to_value(&initial_request)
        .expect("CreateResponseRequest serialization is infallible");

    // Validate: previous_response_id requires store=true per xAI spec.
    // Stateful conversation chaining only works when the server retains
    // the response.
    if initial_request.previous_response_id.is_some() && initial_request.store != Some(true) {
        return Err(ToolLoopError::InvalidConfiguration {
            message: "previous_response_id requires store=true; \
                      stateful conversation chaining only works when the server retains responses"
                .into(),
        });
    }

    // Determine whether to use stateful response-ID chaining (Mode A) or
    // stateless full-conversation rebuild (Mode B).
    //
    // Mode A (stateful): when `store` is `true`, the server keeps the
    // conversation history. Follow-ups chain via `previous_response_id`
    // and only send new tool results in `input`.
    //
    // Mode B (stateless): when `store` is `false`/`None`, the server
    // does NOT keep history. Follow-ups must rebuild the full
    // conversation inline and must NOT set `previous_response_id`.
    let stateful = initial_request.store == Some(true);

    // First request: send exactly as the caller provided (preserving
    // `previous_response_id` if set for conversation continuation).
    let mut response = client.create(&initial_request).await?;
    let mut iterations: u32 = 0;

    // Accumulate the conversation history for Mode B follow-up requests.
    // We serialize the original input so we can rebuild the full context
    // on each iteration without relying on `previous_response_id`.
    let initial_input = serde_json::to_value(&initial_request.input)
        .expect("ResponseInput serialization is infallible");
    let mut conversation: Vec<serde_json::Value> = if stateful {
        // Mode A: we don't need to accumulate the full conversation;
        // it lives on the server.  Start with an empty vec that we'll
        // populate per-iteration with just the new tool outputs.
        Vec::new()
    } else {
        // Mode B: seed from the initial input.
        match &initial_input {
            serde_json::Value::Array(arr) => arr.clone(),
            serde_json::Value::String(s) => vec![serde_json::json!({
                "role": "user",
                "content": s,
            })],
            other => vec![other.clone()],
        }
    };

    loop {
        let calls = extract_function_calls(&response);

        if calls.is_empty() {
            return Ok(response);
        }

        iterations += 1;
        if iterations > config.max_iterations {
            return Err(ToolLoopError::MaxIterationsExceeded {
                iterations,
                max: config.max_iterations,
            });
        }

        // In Mode A we only send new tool results per iteration, so
        // clear the vec before collecting this iteration's outputs.
        // In Mode B we keep accumulating the full conversation.
        if stateful {
            conversation.clear();
        } else {
            // Append only input-compatible output items to the conversation
            // so the model sees its own calls in context.  We use an
            // explicit allowlist so that any new `OutputItem` variants
            // default to being skipped — only variants that are valid
            // as input items in a follow-up request are replayed.
            for item in &response.output {
                match item {
                    OutputItem::Message { .. }
                    | OutputItem::FunctionCall { .. }
                    | OutputItem::FunctionCallOutput { .. } => {
                        conversation.push(
                            serde_json::to_value(item)
                                .expect("OutputItem serialization is infallible"),
                        );
                    }
                    // All other variants — server-side tool calls (McpCall,
                    // WebSearchCall, XSearchCall, CodeInterpreterCall,
                    // FileSearchCall), reasoning traces, unknown, and any
                    // future additions — are NOT valid input items and are
                    // silently skipped.  MCP calls in particular are executed
                    // server-side by the xAI API; they must NOT be replayed
                    // as input items nor passed to the FunctionExecutor.
                    // Using a catch-all ensures new OutputItem variants
                    // default to skipped (fail-safe) without breaking
                    // compilation.
                    _ => {}
                }
            }
        }

        // Execute each function call and collect outputs.
        for call in &calls {
            let result = executor
                .execute(&call.name, &call.arguments)
                .map_err(|error| ToolLoopError::ExecutionFailed {
                    name: call.name.clone(),
                    error,
                })?;

            conversation.push(serde_json::json!({
                "type": "function_call_output",
                "call_id": call.call_id,
                "output": result,
            }));
        }

        // Build the follow-up request.  We use `create_raw` because
        // function_call_output items in the input array are not regular
        // `InputMessage` objects — they have a different schema with
        // `type`, `call_id`, and `output` fields.
        let mut follow_up = initial_json.clone();
        follow_up["input"] = serde_json::json!(conversation);

        if stateful {
            // Mode A: chain via the previous response's ID so the
            // server merges the new tool outputs into its stored
            // conversation context.
            follow_up["previous_response_id"] = serde_json::Value::String(response.id.clone());
        } else {
            // Mode B: remove `previous_response_id` — the first
            // response already incorporated any prior context, and
            // subsequent follow-ups carry the full conversation inline.
            if let Some(obj) = follow_up.as_object_mut() {
                obj.remove("previous_response_id");
            }
        }

        response = client.create_raw(&follow_up).await?;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::responses::{
        CreateResponseBuilder, OutputItem, ResponseInput, ResponseObject, ResponseStatus,
    };

    // -- ToolLoopConfig --

    #[test]
    fn tool_loop_config_default() {
        let config = ToolLoopConfig::default();
        assert_eq!(config.max_iterations, 10);
    }

    #[test]
    fn tool_loop_config_custom() {
        let config = ToolLoopConfig { max_iterations: 5 };
        assert_eq!(config.max_iterations, 5);
    }

    // -- ToolLoopError --

    #[test]
    fn tool_loop_error_display_max_iterations() {
        let err = ToolLoopError::MaxIterationsExceeded {
            iterations: 11,
            max: 10,
        };
        let display = format!("{err}");
        assert!(display.contains("exceeded maximum iterations"));
        assert!(display.contains("11"));
        assert!(display.contains("10"));
    }

    #[test]
    fn tool_loop_error_display_execution_failed() {
        let err = ToolLoopError::ExecutionFailed {
            name: "get_weather".into(),
            error: "network timeout".into(),
        };
        let display = format!("{err}");
        assert!(display.contains("get_weather"));
        assert!(display.contains("network timeout"));
    }

    #[test]
    fn tool_loop_error_display_transport() {
        let transport_err = TransportError::Timeout;
        let err = ToolLoopError::Transport(transport_err);
        let display = format!("{err}");
        assert!(display.contains("transport error"));
        assert!(display.contains("timed out"));
    }

    #[test]
    fn tool_loop_error_from_transport() {
        let transport_err = TransportError::Timeout;
        let err: ToolLoopError = transport_err.into();
        assert!(matches!(
            err,
            ToolLoopError::Transport(TransportError::Timeout)
        ));
    }

    #[test]
    fn tool_loop_error_is_std_error() {
        let err = ToolLoopError::MaxIterationsExceeded {
            iterations: 1,
            max: 1,
        };
        let _: &dyn std::error::Error = &err;
    }

    // -- extract_function_calls --

    #[test]
    fn extract_function_calls_finds_calls() {
        let response = ResponseObject {
            id: "resp_1".into(),
            object: None,
            status: ResponseStatus::Completed,
            output: vec![
                OutputItem::FunctionCall {
                    id: "fc_1".into(),
                    call_id: "call_1".into(),
                    name: "get_weather".into(),
                    arguments: r#"{"city":"NYC"}"#.into(),
                },
                OutputItem::Message {
                    role: crate::types::common::Role::Assistant,
                    content: vec![],
                },
                OutputItem::FunctionCall {
                    id: "fc_2".into(),
                    call_id: "call_2".into(),
                    name: "get_time".into(),
                    arguments: r#"{"tz":"UTC"}"#.into(),
                },
            ],
            usage: None,
            model: None,
            instructions: None,
            metadata: None,
            previous_response_id: None,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
        };

        let calls = extract_function_calls(&response);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].call_id, "call_1");
        assert_eq!(calls[1].name, "get_time");
        assert_eq!(calls[1].call_id, "call_2");
    }

    #[test]
    fn extract_function_calls_empty_when_no_calls() {
        let response = ResponseObject {
            id: "resp_2".into(),
            object: None,
            status: ResponseStatus::Completed,
            output: vec![OutputItem::Message {
                role: crate::types::common::Role::Assistant,
                content: vec![crate::types::common::ContentBlock::Text {
                    text: "Hello!".into(),
                }],
            }],
            usage: None,
            model: None,
            instructions: None,
            metadata: None,
            previous_response_id: None,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
        };

        let calls = extract_function_calls(&response);
        assert!(calls.is_empty());
    }

    // -- FunctionExecutor mock --

    struct MockExecutor {
        responses: std::collections::HashMap<String, String>,
    }

    impl MockExecutor {
        fn new() -> Self {
            let mut responses = std::collections::HashMap::new();
            responses.insert("get_weather".into(), r#"{"temp": 72, "unit": "F"}"#.into());
            responses.insert(
                "get_time".into(),
                r#"{"time": "12:00", "tz": "UTC"}"#.into(),
            );
            Self { responses }
        }
    }

    impl FunctionExecutor for MockExecutor {
        fn execute(
            &self,
            name: &str,
            _arguments: &str,
        ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
            self.responses
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown function: {name}").into())
        }
    }

    #[test]
    fn mock_executor_returns_known_function() {
        let executor = MockExecutor::new();
        let result = executor.execute("get_weather", "{}").unwrap();
        assert!(result.contains("72"));
    }

    #[test]
    fn mock_executor_errors_on_unknown_function() {
        let executor = MockExecutor::new();
        let result = executor.execute("unknown", "{}");
        assert!(result.is_err());
    }

    // -- Integration test with wiremock --

    #[tokio::test]
    async fn tool_loop_drives_call_execute_return_cycle() {
        use crate::transport::auth::ApiKeySecret;
        use crate::transport::client::{HttpClient, HttpClientConfig};
        use std::sync::Arc;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // First response: model wants to call a function
        let first_response = serde_json::json!({
            "id": "resp_1",
            "status": "completed",
            "output": [
                {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_abc",
                    "name": "get_weather",
                    "arguments": "{\"city\":\"NYC\"}"
                }
            ]
        });

        // Second response: model produces final text
        let second_response = serde_json::json!({
            "id": "resp_2",
            "status": "completed",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "text", "text": "It's 72F in NYC."}]
                }
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&first_response))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&second_response))
            .mount(&server)
            .await;

        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let client = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(crate::transport::policy_gate::AllowAllGate)),
        )
        .unwrap();
        let responses_client = ResponsesClient::new(Arc::new(client));

        let initial = CreateResponseBuilder::new(
            "grok-4",
            ResponseInput::Text("What's the weather in NYC?".into()),
        )
        .tools(vec![serde_json::json!({
            "type": "function",
            "name": "get_weather",
            "description": "Get weather",
            "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}
        })])
        .build();

        let executor = MockExecutor::new();
        let loop_config = ToolLoopConfig { max_iterations: 5 };

        let result = run_tool_loop(&responses_client, initial, &executor, loop_config)
            .await
            .unwrap();

        assert_eq!(result.id, "resp_2");
        assert_eq!(result.output.len(), 1);
        match &result.output[0] {
            OutputItem::Message { content, .. } => {
                let text = match &content[0] {
                    crate::types::common::ContentBlock::Text { text } => text,
                    other => panic!("expected Text block, got: {other:?}"),
                };
                assert!(text.contains("72F"));
            }
            other => panic!("expected Message, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_loop_respects_max_iterations() {
        use crate::transport::auth::ApiKeySecret;
        use crate::transport::client::{HttpClient, HttpClientConfig};
        use std::sync::Arc;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // Every response has a function call — the loop should never terminate
        // naturally and should hit the max iterations limit.
        let looping_response = serde_json::json!({
            "id": "resp_loop",
            "status": "completed",
            "output": [
                {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_loop",
                    "name": "get_weather",
                    "arguments": "{}"
                }
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&looping_response))
            .mount(&server)
            .await;

        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let client = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(crate::transport::policy_gate::AllowAllGate)),
        )
        .unwrap();
        let responses_client = ResponsesClient::new(Arc::new(client));

        let initial =
            CreateResponseBuilder::new("grok-4", ResponseInput::Text("loop forever".into()))
                .tools(vec![serde_json::json!({
                    "type": "function",
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": {}
                })])
                .build();

        let executor = MockExecutor::new();
        let loop_config = ToolLoopConfig { max_iterations: 2 };

        let result = run_tool_loop(&responses_client, initial, &executor, loop_config).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ToolLoopError::MaxIterationsExceeded { iterations, max } => {
                assert_eq!(iterations, 3);
                assert_eq!(max, 2);
            }
            other => panic!("expected MaxIterationsExceeded, got: {other}"),
        }
    }

    #[tokio::test]
    async fn tool_loop_returns_execution_error() {
        use crate::transport::auth::ApiKeySecret;
        use crate::transport::client::{HttpClient, HttpClientConfig};
        use std::sync::Arc;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // Response calls an unknown function
        let response = serde_json::json!({
            "id": "resp_err",
            "status": "completed",
            "output": [
                {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_unknown",
                    "name": "nonexistent_function",
                    "arguments": "{}"
                }
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let client = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(crate::transport::policy_gate::AllowAllGate)),
        )
        .unwrap();
        let responses_client = ResponsesClient::new(Arc::new(client));

        let initial =
            CreateResponseBuilder::new("grok-4", ResponseInput::Text("call unknown".into()))
                .build();

        let executor = MockExecutor::new();
        let loop_config = ToolLoopConfig::default();

        let result = run_tool_loop(&responses_client, initial, &executor, loop_config).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ToolLoopError::ExecutionFailed { name, .. } => {
                assert_eq!(name, "nonexistent_function");
            }
            other => panic!("expected ExecutionFailed, got: {other}"),
        }
    }

    #[tokio::test]
    async fn tool_loop_immediate_response_no_calls() {
        use crate::transport::auth::ApiKeySecret;
        use crate::transport::client::{HttpClient, HttpClientConfig};
        use std::sync::Arc;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // Response with no function calls — should return immediately
        let response = serde_json::json!({
            "id": "resp_immediate",
            "status": "completed",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "text", "text": "No tools needed."}]
                }
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .expect(1)
            .mount(&server)
            .await;

        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let client = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(crate::transport::policy_gate::AllowAllGate)),
        )
        .unwrap();
        let responses_client = ResponsesClient::new(Arc::new(client));

        let initial =
            CreateResponseBuilder::new("grok-4", ResponseInput::Text("Hello".into())).build();

        let executor = MockExecutor::new();
        let loop_config = ToolLoopConfig::default();

        let result = run_tool_loop(&responses_client, initial, &executor, loop_config)
            .await
            .unwrap();

        assert_eq!(result.id, "resp_immediate");
    }

    // -- Mode A (stateful) vs Mode B (stateless) --

    /// Helper: sets up a wiremock server that records request bodies.
    /// Returns (server, Arc<Mutex<Vec<Value>>>) where the vec collects
    /// all request bodies received.
    async fn setup_recording_server() -> (
        wiremock::MockServer,
        std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    ) {
        use std::sync::{Arc, Mutex};
        use wiremock::MockServer;

        let server = MockServer::start().await;
        let bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        (server, bodies)
    }

    /// Custom wiremock responder that captures request bodies and cycles
    /// through a list of canned responses.  Uses `std::sync::Mutex`
    /// because wiremock's `Respond` trait is synchronous.
    #[derive(Clone)]
    struct RecordingResponder {
        bodies: std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
        responses: std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    }

    impl wiremock::Respond for RecordingResponder {
        fn respond(&self, request: &wiremock::Request) -> wiremock::ResponseTemplate {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or_default();
            self.bodies.lock().unwrap().push(body);
            let resp = {
                let mut resps = self.responses.lock().unwrap();
                if resps.len() > 1 {
                    resps.remove(0)
                } else {
                    resps[0].clone()
                }
            };
            wiremock::ResponseTemplate::new(200).set_body_json(resp)
        }
    }

    #[tokio::test]
    async fn tool_loop_mode_a_stateful_chains_response_ids() {
        use crate::transport::auth::ApiKeySecret;
        use crate::transport::client::{HttpClient, HttpClientConfig};
        use std::sync::Arc;
        use wiremock::Mock;
        use wiremock::matchers::{method, path};

        let (server, bodies) = setup_recording_server().await;

        let responses = Arc::new(std::sync::Mutex::new(vec![
            // First response: function call
            serde_json::json!({
                "id": "resp_first",
                "status": "completed",
                "output": [{
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "get_weather",
                    "arguments": "{\"city\":\"NYC\"}"
                }]
            }),
            // Second response: another function call (to test chaining across 2 iterations)
            serde_json::json!({
                "id": "resp_second",
                "status": "completed",
                "output": [{
                    "type": "function_call",
                    "id": "fc_2",
                    "call_id": "call_2",
                    "name": "get_time",
                    "arguments": "{\"tz\":\"UTC\"}"
                }]
            }),
            // Third response: final text
            serde_json::json!({
                "id": "resp_final",
                "status": "completed",
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "text", "text": "Done."}]
                }]
            }),
        ]));

        let responder = RecordingResponder {
            bodies: bodies.clone(),
            responses,
        };
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(responder)
            .mount(&server)
            .await;

        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let client = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(crate::transport::policy_gate::AllowAllGate)),
        )
        .unwrap();
        let responses_client = ResponsesClient::new(Arc::new(client));

        // Mode A: store=true, with a previous_response_id
        let initial =
            CreateResponseBuilder::new("grok-4", ResponseInput::Text("What's the weather?".into()))
                .store(true)
                .previous_response_id("resp_earlier")
                .tools(vec![serde_json::json!({
                    "type": "function",
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": {}
                })])
                .build();

        let executor = MockExecutor::new();
        let loop_config = ToolLoopConfig { max_iterations: 5 };

        let result = run_tool_loop(&responses_client, initial, &executor, loop_config)
            .await
            .unwrap();

        assert_eq!(result.id, "resp_final");

        let captured = bodies.lock().unwrap();
        assert_eq!(
            captured.len(),
            3,
            "expected 3 requests (initial + 2 follow-ups)"
        );

        // Request 0: initial — should have the caller's previous_response_id
        assert_eq!(
            captured[0]["previous_response_id"].as_str(),
            Some("resp_earlier"),
            "initial request should preserve caller's previous_response_id"
        );

        // Request 1: first follow-up — should chain to resp_first
        assert_eq!(
            captured[1]["previous_response_id"].as_str(),
            Some("resp_first"),
            "first follow-up should chain to resp_first"
        );
        // Input should contain ONLY the tool output, not the full conversation
        let input_1 = captured[1]["input"].as_array().unwrap();
        assert_eq!(
            input_1.len(),
            1,
            "Mode A follow-up should have only tool output"
        );
        assert_eq!(input_1[0]["type"], "function_call_output");
        assert_eq!(input_1[0]["call_id"], "call_1");

        // Request 2: second follow-up — should chain to resp_second
        assert_eq!(
            captured[2]["previous_response_id"].as_str(),
            Some("resp_second"),
            "second follow-up should chain to resp_second"
        );
        let input_2 = captured[2]["input"].as_array().unwrap();
        assert_eq!(
            input_2.len(),
            1,
            "Mode A follow-up should have only tool output"
        );
        assert_eq!(input_2[0]["type"], "function_call_output");
        assert_eq!(input_2[0]["call_id"], "call_2");
    }

    #[tokio::test]
    async fn tool_loop_mode_b_stateless_rebuilds_full_conversation() {
        use crate::transport::auth::ApiKeySecret;
        use crate::transport::client::{HttpClient, HttpClientConfig};
        use std::sync::Arc;
        use wiremock::Mock;
        use wiremock::matchers::{method, path};

        let (server, bodies) = setup_recording_server().await;

        let responses = Arc::new(std::sync::Mutex::new(vec![
            // First response: function call
            serde_json::json!({
                "id": "resp_1",
                "status": "completed",
                "output": [{
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_abc",
                    "name": "get_weather",
                    "arguments": "{\"city\":\"NYC\"}"
                }]
            }),
            // Second response: final text
            serde_json::json!({
                "id": "resp_2",
                "status": "completed",
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "text", "text": "It's 72F."}]
                }]
            }),
        ]));

        let responder = RecordingResponder {
            bodies: bodies.clone(),
            responses,
        };
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(responder)
            .mount(&server)
            .await;

        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let client = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(crate::transport::policy_gate::AllowAllGate)),
        )
        .unwrap();
        let responses_client = ResponsesClient::new(Arc::new(client));

        // Mode B: store=false (default)
        let initial =
            CreateResponseBuilder::new("grok-4", ResponseInput::Text("What's the weather?".into()))
                .tools(vec![serde_json::json!({
                    "type": "function",
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": {}
                })])
                .build();

        assert_eq!(
            initial.store,
            Some(false),
            "builder default should be store=false"
        );

        let executor = MockExecutor::new();
        let loop_config = ToolLoopConfig { max_iterations: 5 };

        let result = run_tool_loop(&responses_client, initial, &executor, loop_config)
            .await
            .unwrap();

        assert_eq!(result.id, "resp_2");

        let captured = bodies.lock().unwrap();
        assert_eq!(
            captured.len(),
            2,
            "expected 2 requests (initial + 1 follow-up)"
        );

        // Follow-up should NOT have previous_response_id
        assert!(
            captured[1].get("previous_response_id").is_none(),
            "Mode B follow-up must not include previous_response_id"
        );

        // Follow-up input should contain the full conversation:
        // original user message + assistant function call + tool output
        let input = captured[1]["input"].as_array().unwrap();
        assert!(
            input.len() >= 3,
            "Mode B follow-up should rebuild full conversation, got {} items",
            input.len()
        );

        // First item: original user message
        assert_eq!(input[0]["role"], "user");
        // Second item: assistant's function call
        assert_eq!(input[1]["type"], "function_call");
        // Third item: tool output
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_abc");
    }

    /// Stateless Mode B must only replay input-compatible `OutputItem` variants
    /// (Message, FunctionCall, FunctionCallOutput) and skip server-side tool
    /// calls, reasoning traces, and unknown items.
    #[tokio::test]
    async fn tool_loop_mode_b_filters_non_input_output_items() {
        use crate::transport::auth::ApiKeySecret;
        use crate::transport::client::{HttpClient, HttpClientConfig};
        use std::sync::Arc;
        use wiremock::Mock;
        use wiremock::matchers::{method, path};

        let (server, bodies) = setup_recording_server().await;

        let responses = Arc::new(std::sync::Mutex::new(vec![
            // First response: function call mixed with non-input items
            serde_json::json!({
                "id": "resp_1",
                "status": "completed",
                "output": [
                    {
                        "type": "reasoning",
                        "id": "reasoning_1",
                        "content": [{"type": "thinking", "text": "Let me think..."}]
                    },
                    {
                        "type": "web_search_call",
                        "id": "ws_1",
                        "status": "completed"
                    },
                    {
                        "type": "function_call",
                        "id": "fc_1",
                        "call_id": "call_abc",
                        "name": "get_weather",
                        "arguments": "{\"city\":\"NYC\"}"
                    }
                ]
            }),
            // Second response: final text
            serde_json::json!({
                "id": "resp_2",
                "status": "completed",
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "text", "text": "It's 72F."}]
                }]
            }),
        ]));

        let responder = RecordingResponder {
            bodies: bodies.clone(),
            responses,
        };
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(responder)
            .mount(&server)
            .await;

        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let client = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(crate::transport::policy_gate::AllowAllGate)),
        )
        .unwrap();
        let responses_client = ResponsesClient::new(Arc::new(client));

        // Mode B: store=false (default)
        let initial =
            CreateResponseBuilder::new("grok-4", ResponseInput::Text("What's the weather?".into()))
                .tools(vec![serde_json::json!({
                    "type": "function",
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": {}
                })])
                .build();

        let executor = MockExecutor::new();
        let loop_config = ToolLoopConfig { max_iterations: 5 };

        let result = run_tool_loop(&responses_client, initial, &executor, loop_config)
            .await
            .unwrap();

        assert_eq!(result.id, "resp_2");

        let captured = bodies.lock().unwrap();
        assert_eq!(
            captured.len(),
            2,
            "expected 2 requests (initial + 1 follow-up)"
        );

        // The follow-up input should contain:
        //   [0] original user message
        //   [1] the function_call (input-compatible)
        //   [2] the function_call_output (tool result)
        // The reasoning and web_search_call items must NOT appear.
        let input = captured[1]["input"].as_array().unwrap();
        assert_eq!(
            input.len(),
            3,
            "expected 3 items in follow-up input (user msg + function_call + function_call_output), \
             got {}: non-input items should have been filtered out",
            input.len()
        );

        // Verify none of the filtered-out types leaked through.
        for item in input {
            let item_type = item
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            assert!(
                ![
                    "reasoning",
                    "web_search_call",
                    "x_search_call",
                    "code_interpreter_call",
                    "file_search_call",
                    "mcp_call"
                ]
                .contains(&item_type),
                "non-input item type '{item_type}' must not appear in stateless follow-up input"
            );
        }

        // Verify the expected items are present and in order.
        assert_eq!(
            input[0]["role"], "user",
            "first item should be user message"
        );
        assert_eq!(
            input[1]["type"], "function_call",
            "second item should be function_call"
        );
        assert_eq!(input[1]["call_id"], "call_abc");
        assert_eq!(
            input[2]["type"], "function_call_output",
            "third item should be function_call_output"
        );
        assert_eq!(input[2]["call_id"], "call_abc");
    }

    // -- MCP call handling --

    #[test]
    fn extract_function_calls_ignores_mcp_call_items() {
        let response = ResponseObject {
            id: "resp_mcp".into(),
            object: None,
            status: ResponseStatus::Completed,
            output: vec![
                // MCP call — server-side, should NOT be extracted
                OutputItem::McpCall {
                    id: "mcp_1".into(),
                    status: Some("completed".into()),
                },
                // Function call — client-side, SHOULD be extracted
                OutputItem::FunctionCall {
                    id: "fc_1".into(),
                    call_id: "call_1".into(),
                    name: "get_weather".into(),
                    arguments: r#"{"city":"NYC"}"#.into(),
                },
                // Another MCP call
                OutputItem::McpCall {
                    id: "mcp_2".into(),
                    status: Some("completed".into()),
                },
                // Another function call
                OutputItem::FunctionCall {
                    id: "fc_2".into(),
                    call_id: "call_2".into(),
                    name: "get_time".into(),
                    arguments: r#"{"tz":"UTC"}"#.into(),
                },
                // Web search call — also server-side, should be ignored
                OutputItem::WebSearchCall {
                    id: "ws_1".into(),
                    status: Some("completed".into()),
                    search_results: None,
                },
            ],
            usage: None,
            model: None,
            instructions: None,
            metadata: None,
            previous_response_id: None,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
        };

        let calls = extract_function_calls(&response);
        assert_eq!(
            calls.len(),
            2,
            "only FunctionCall items should be extracted, not McpCall or WebSearchCall"
        );
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].call_id, "call_1");
        assert_eq!(calls[1].name, "get_time");
        assert_eq!(calls[1].call_id, "call_2");
    }

    /// Verify that in stateless Mode B, McpCall items in the response output
    /// are NOT replayed into the follow-up request's input array, while
    /// FunctionCall items ARE replayed.
    #[tokio::test]
    async fn tool_loop_mode_b_does_not_replay_mcp_call_items() {
        use crate::transport::auth::ApiKeySecret;
        use crate::transport::client::{HttpClient, HttpClientConfig};
        use std::sync::Arc;
        use wiremock::Mock;
        use wiremock::matchers::{method, path};

        let (server, bodies) = setup_recording_server().await;

        let responses = Arc::new(std::sync::Mutex::new(vec![
            // First response: MCP call + function call
            serde_json::json!({
                "id": "resp_1",
                "status": "completed",
                "output": [
                    {
                        "type": "mcp_call",
                        "id": "mcp_1",
                        "status": "completed"
                    },
                    {
                        "type": "function_call",
                        "id": "fc_1",
                        "call_id": "call_abc",
                        "name": "get_weather",
                        "arguments": "{\"city\":\"NYC\"}"
                    }
                ]
            }),
            // Second response: final text
            serde_json::json!({
                "id": "resp_2",
                "status": "completed",
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "text", "text": "Done."}]
                }]
            }),
        ]));

        let responder = RecordingResponder {
            bodies: bodies.clone(),
            responses,
        };
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(responder)
            .mount(&server)
            .await;

        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let client = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(crate::transport::policy_gate::AllowAllGate)),
        )
        .unwrap();
        let responses_client = ResponsesClient::new(Arc::new(client));

        // Mode B: store=false (default)
        let initial =
            CreateResponseBuilder::new("grok-4", ResponseInput::Text("Use MCP and tools".into()))
                .tools(vec![serde_json::json!({
                    "type": "function",
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": {}
                })])
                .build();

        let executor = MockExecutor::new();
        let loop_config = ToolLoopConfig { max_iterations: 5 };

        let result = run_tool_loop(&responses_client, initial, &executor, loop_config)
            .await
            .unwrap();

        assert_eq!(result.id, "resp_2");

        let captured = bodies.lock().unwrap();
        assert_eq!(captured.len(), 2, "expected initial + 1 follow-up");

        // The follow-up input should contain:
        //   [0] original user message
        //   [1] the function_call (input-compatible, replayed)
        //   [2] the function_call_output (tool result)
        // The mcp_call item must NOT appear.
        let input = captured[1]["input"].as_array().unwrap();
        assert_eq!(
            input.len(),
            3,
            "expected user msg + function_call + function_call_output; mcp_call must be filtered"
        );

        // Verify no mcp_call leaked through
        for item in input {
            let item_type = item
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            assert_ne!(
                item_type, "mcp_call",
                "mcp_call items must not be replayed in stateless mode"
            );
        }

        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "call_abc");
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_abc");
    }

    /// `previous_response_id` without `store=true` is an invalid configuration
    /// per the xAI spec. The tool loop must reject this upfront.
    #[tokio::test]
    async fn tool_loop_previous_response_id_without_store_returns_error() {
        use crate::transport::auth::ApiKeySecret;
        use crate::transport::client::{HttpClient, HttpClientConfig};
        use std::sync::Arc;

        let server = wiremock::MockServer::start().await;
        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let client = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(crate::transport::policy_gate::AllowAllGate)),
        )
        .unwrap();
        let responses_client = ResponsesClient::new(Arc::new(client));

        // previous_response_id set, but store is false (default)
        let initial = CreateResponseBuilder::new(
            "grok-4",
            ResponseInput::Text("Continue the conversation".into()),
        )
        .previous_response_id("resp_earlier")
        // store defaults to false — NOT setting store(true)
        .build();

        let executor = MockExecutor::new();
        let loop_config = ToolLoopConfig { max_iterations: 5 };

        let result = run_tool_loop(&responses_client, initial, &executor, loop_config).await;
        assert!(result.is_err(), "should fail with InvalidConfiguration");
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("previous_response_id requires store=true"),
            "error message should explain the issue, got: {msg}"
        );
    }
}
