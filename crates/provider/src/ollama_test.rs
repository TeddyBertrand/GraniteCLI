use super::*;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn chat_req() -> ChatRequest {
    ChatRequest {
        messages: vec![user_msg("hi")],
        tools: vec![],
        model: "llama3.1".to_string(),
    }
}

fn user_msg(text: &str) -> Message {
    Message {
        role: Role::User,
        content: Some(text.to_string()),
        tool_calls: vec![],
        tool_call_id: None,
    }
}

#[test]
fn request_omits_empty_tools() {
    let req = chat_req();
    let body = OllamaRequest::from_chat_request(&req, false);
    let value = serde_json::to_value(&body).unwrap();

    assert!(value.get("tools").is_none(), "empty tools should be omitted");
    let msg = &value["messages"][0];
    assert!(msg.get("tool_calls").is_none(), "empty tool_calls should be omitted");
    assert_eq!(msg["content"], "hi");
    assert_eq!(value["stream"], false);
}

#[test]
fn request_carries_tool_definitions_and_stream_flag() {
    let req = ChatRequest {
        messages: vec![user_msg("hi")],
        tools: vec![ToolDefinition {
            name: "bash".to_string(),
            description: "run a shell command".to_string(),
            parameters: json!({"type": "object", "properties": {}}),
        }],
        model: "llama3.1".to_string(),
    };
    let body = OllamaRequest::from_chat_request(&req, true);
    let value = serde_json::to_value(&body).unwrap();

    assert_eq!(value["stream"], true);
    assert_eq!(value["tools"][0]["type"], "function");
    assert_eq!(value["tools"][0]["function"]["name"], "bash");
}

#[test]
fn tool_call_arguments_round_trip_as_json_value() {
    let tc = ToolCall {
        id: "call_1".to_string(),
        name: "bash".to_string(),
        arguments: json!({"cmd": "ls"}),
        provider_metadata: None,
    };
    let wire = OllamaToolCall::from_tool_call(&tc);
    assert_eq!(wire.function.arguments, json!({"cmd": "ls"}));

    let back = wire.into_tool_call();
    assert_eq!(back, tc);
}

#[test]
fn response_without_tool_calls_maps_to_stop() {
    let raw = json!({
        "message": { "role": "assistant", "content": "hello", "tool_calls": [] },
        "done": true,
        "done_reason": "stop"
    });
    let parsed: OllamaChatChunk = serde_json::from_value(raw).unwrap();
    let resp = parsed.into_chat_response();
    assert_eq!(resp.message.content.as_deref(), Some("hello"));
    assert_eq!(resp.finish_reason, FinishReason::Stop);
}

#[test]
fn response_with_tool_calls_maps_to_tool_calls_finish_reason() {
    let raw = json!({
        "message": {
            "role": "assistant",
            "content": null,
            "tool_calls": [{"function": {"name": "bash", "arguments": {"cmd": "ls"}}}]
        },
        "done": true
    });
    let parsed: OllamaChatChunk = serde_json::from_value(raw).unwrap();
    let resp = parsed.into_chat_response();
    assert_eq!(resp.finish_reason, FinishReason::ToolCalls);
    assert_eq!(resp.message.tool_calls[0].name, "bash");
}

#[test]
fn ndjson_line_not_done_yields_no_finish_reason() {
    let line = br#"{"message":{"role":"assistant","content":"he"},"done":false}
"#;
    let parsed = parse_line(line).expect("should yield an item").unwrap();
    assert_eq!(parsed.delta.as_deref(), Some("he"));
    assert!(parsed.finish_reason.is_none());
}

#[test]
fn ndjson_final_line_reports_finish_reason() {
    let line = br#"{"message":{"role":"assistant","content":""},"done":true,"done_reason":"stop"}
"#;
    let parsed = parse_line(line).expect("should yield an item").unwrap();
    assert_eq!(parsed.finish_reason, Some(FinishReason::Stop));
}

#[test]
fn take_line_splits_on_newline_and_buffers_partial() {
    let mut buf = b"{\"done\":false}\n{\"partial".to_vec();
    let line = take_line(&mut buf).expect("first line available");
    assert_eq!(line, b"{\"done\":false}\n");
    assert_eq!(buf, b"{\"partial");
    assert!(take_line(&mut buf).is_none());
}

#[tokio::test]
async fn chat_sends_request_and_parses_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": { "role": "assistant", "content": "hi back", "tool_calls": [] },
            "done": true,
            "done_reason": "stop"
        })))
        .mount(&server)
        .await;

    let provider = OllamaProvider::new().with_base_url(server.uri());
    let resp = provider.chat(chat_req()).await.unwrap();

    assert_eq!(resp.message.content.as_deref(), Some("hi back"));
    assert_eq!(resp.finish_reason, FinishReason::Stop);
}

#[tokio::test]
async fn chat_500_maps_to_other_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let provider = OllamaProvider::new().with_base_url(server.uri());
    let err = provider.chat(chat_req()).await.unwrap_err();
    assert!(matches!(err, ProviderError::Other(msg) if msg.contains("boom")));
}

#[tokio::test]
async fn chat_429_maps_to_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;

    let provider = OllamaProvider::new().with_base_url(server.uri());
    let err = provider.chat(chat_req()).await.unwrap_err();
    assert!(matches!(err, ProviderError::RateLimited { .. }));
}

#[tokio::test]
async fn chat_stream_yields_content_deltas_over_ndjson() {
    let server = MockServer::start().await;
    let body = concat!(
        "{\"message\":{\"role\":\"assistant\",\"content\":\"he\"},\"done\":false}\n",
        "{\"message\":{\"role\":\"assistant\",\"content\":\"llo\"},\"done\":true,\"done_reason\":\"stop\"}\n",
    );
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/x-ndjson"))
        .mount(&server)
        .await;

    let provider = OllamaProvider::new().with_base_url(server.uri());
    let mut stream = provider.chat_stream(chat_req()).await.unwrap();

    let first = stream.next().await.unwrap().unwrap();
    assert_eq!(first.delta.as_deref(), Some("he"));
    assert!(first.finish_reason.is_none());

    let second = stream.next().await.unwrap().unwrap();
    assert_eq!(second.delta.as_deref(), Some("llo"));
    assert_eq!(second.finish_reason, Some(FinishReason::Stop));

    assert!(stream.next().await.is_none());
}

#[test]
fn from_env_defaults_to_localhost() {
    // SAFETY: test runs single-threaded within this process's env mutation window;
    // no other test reads/writes OLLAMA_BASE_URL.
    unsafe { env::remove_var("OLLAMA_BASE_URL") };
    let provider = OllamaProvider::from_env();
    assert_eq!(provider.base_url, DEFAULT_BASE_URL);
}
