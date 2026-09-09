use super::*;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn chat_req() -> ChatRequest {
    ChatRequest {
        messages: vec![user_msg("hi")],
        tools: vec![],
        model: "llama-3.3-70b-versatile".to_string(),
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
fn request_omits_empty_tools_and_tool_calls() {
    let req = ChatRequest {
        messages: vec![user_msg("hi")],
        tools: vec![],
        model: "llama-3.3-70b-versatile".to_string(),
    };
    let body = GroqRequest::from_chat_request(&req, false);
    let value = serde_json::to_value(&body).unwrap();

    assert!(value.get("tools").is_none(), "empty tools should be omitted");
    let msg = &value["messages"][0];
    assert!(msg.get("tool_calls").is_none(), "empty tool_calls should be omitted");
    assert!(msg.get("tool_call_id").is_none());
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
        model: "llama-3.3-70b-versatile".to_string(),
    };
    let body = GroqRequest::from_chat_request(&req, true);
    let value = serde_json::to_value(&body).unwrap();

    assert_eq!(value["stream"], true);
    assert_eq!(value["tools"][0]["type"], "function");
    assert_eq!(value["tools"][0]["function"]["name"], "bash");
}

#[test]
fn tool_call_arguments_round_trip_through_json_string() {
    let tc = ToolCall {
        id: "call_1".to_string(),
        name: "bash".to_string(),
        arguments: json!({"cmd": "ls"}),
        provider_metadata: None,
    };
    let wire = GroqToolCall::from_tool_call(&tc);
    assert_eq!(wire.function.arguments, r#"{"cmd":"ls"}"#);

    let back = wire.into_tool_call();
    assert_eq!(back, tc);
}

#[test]
fn tool_call_arguments_survive_malformed_json() {
    let wire = GroqToolCall {
        id: "call_2".to_string(),
        kind: "function".to_string(),
        function: GroqFunctionCall {
            name: "bash".to_string(),
            arguments: "not json".to_string(),
        },
    };
    let tc = wire.into_tool_call();
    assert_eq!(tc.arguments, serde_json::Value::String("not json".to_string()));
}

#[test]
fn response_parses_into_chat_response() {
    let raw = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "hello",
                "tool_calls": []
            },
            "finish_reason": "stop"
        }]
    });
    let mut parsed: GroqResponse = serde_json::from_value(raw).unwrap();
    let choice = parsed.choices.remove(0);
    let resp = ChatResponse {
        message: choice.message.into_message(),
        finish_reason: choice.finish_reason.into(),
    };
    assert_eq!(resp.message.content.as_deref(), Some("hello"));
    assert_eq!(resp.finish_reason, FinishReason::Stop);
}

#[test]
fn sse_event_parses_content_delta() {
    let event = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n";
    let parsed = parse_event(event).expect("should yield an item").unwrap();
    assert_eq!(parsed.delta.as_deref(), Some("hi"));
    assert!(parsed.finish_reason.is_none());
}

#[test]
fn sse_done_marker_yields_no_item() {
    let event = b"data: [DONE]\n\n";
    assert!(parse_event(event).is_none());
}

#[test]
fn sse_event_reports_finish_reason() {
    let event = b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
    let parsed = parse_event(event).expect("should yield an item").unwrap();
    assert_eq!(parsed.finish_reason, Some(FinishReason::Stop));
}

#[test]
fn take_event_splits_on_double_newline_and_buffers_partial() {
    let mut buf = b"data: {\"choices\":[]}\n\ndata: partial".to_vec();
    let event = take_event(&mut buf).expect("first event available");
    assert_eq!(event, b"data: {\"choices\":[]}\n\n");
    assert_eq!(buf, b"data: partial");
    assert!(take_event(&mut buf).is_none());
}

#[tokio::test]
async fn chat_sends_bearer_auth_and_parses_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": { "role": "assistant", "content": "hi back", "tool_calls": [] },
                "finish_reason": "stop"
            }]
        })))
        .mount(&server)
        .await;

    let provider = GroqProvider::new("test-key").with_base_url(server.uri());
    let resp = provider.chat(chat_req()).await.unwrap();

    assert_eq!(resp.message.content.as_deref(), Some("hi back"));
    assert_eq!(resp.finish_reason, FinishReason::Stop);
}

#[tokio::test]
async fn chat_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let provider = GroqProvider::new("bad-key").with_base_url(server.uri());
    let err = provider.chat(chat_req()).await.unwrap_err();
    assert!(matches!(err, ProviderError::Auth));
}

#[tokio::test]
async fn chat_429_maps_to_rate_limited_with_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "7"))
        .mount(&server)
        .await;

    let provider = GroqProvider::new("test-key").with_base_url(server.uri());
    let err = provider.chat(chat_req()).await.unwrap_err();
    match err {
        ProviderError::RateLimited { retry_after } => {
            assert_eq!(retry_after, Some(Duration::from_secs(7)));
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

#[tokio::test]
async fn chat_500_maps_to_other_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let provider = GroqProvider::new("test-key").with_base_url(server.uri());
    let err = provider.chat(chat_req()).await.unwrap_err();
    assert!(matches!(err, ProviderError::Other(msg) if msg.contains("boom")));
}

#[tokio::test]
async fn chat_stream_yields_content_deltas_over_sse() {
    let server = MockServer::start().await;
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"he\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"llo\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;

    let provider = GroqProvider::new("test-key").with_base_url(server.uri());
    let mut stream = provider.chat_stream(chat_req()).await.unwrap();

    let first = stream.next().await.unwrap().unwrap();
    assert_eq!(first.delta.as_deref(), Some("he"));

    let second = stream.next().await.unwrap().unwrap();
    assert_eq!(second.delta.as_deref(), Some("llo"));
    assert_eq!(second.finish_reason, Some(FinishReason::Stop));

    assert!(stream.next().await.is_none());
}

#[test]
fn from_env_without_api_key_is_auth_error() {
    // SAFETY: test runs single-threaded within this process's env mutation window;
    // no other test reads/writes GROQ_API_KEY.
    unsafe { env::remove_var("GROQ_API_KEY") };
    let result = GroqProvider::from_env();
    assert!(matches!(result, Err(ProviderError::Auth)));
}
