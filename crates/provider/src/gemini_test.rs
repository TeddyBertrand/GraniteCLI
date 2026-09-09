use super::*;
use crate::traits::{ChatRequest, Message, Role, ToolCall, ToolDefinition};

fn req_with_system_and_user() -> ChatRequest {
    ChatRequest {
        model: "gemini-1.5-flash".to_string(),
        tools: vec![],
        messages: vec![
            Message {
                role: Role::System,
                content: Some("be terse".to_string()),
                tool_calls: vec![],
                tool_call_id: None,
            },
            Message {
                role: Role::User,
                content: Some("hi".to_string()),
                tool_calls: vec![],
                tool_call_id: None,
            },
        ],
    }
}

#[test]
fn system_message_routes_to_system_instruction() {
    let body = GeminiRequest::from_chat_request(&req_with_system_and_user());

    let sys = body.system_instruction.expect("system_instruction present");
    assert!(matches!(&sys.parts[0], GeminiPart::Text { text } if text == "be terse"));

    assert_eq!(body.contents.len(), 1);
    assert_eq!(body.contents[0].role.as_deref(), Some("user"));
}

#[test]
fn assistant_role_maps_to_model() {
    let req = ChatRequest {
        model: "gemini-1.5-flash".to_string(),
        tools: vec![],
        messages: vec![Message {
            role: Role::Assistant,
            content: Some("hello".to_string()),
            tool_calls: vec![],
            tool_call_id: None,
        }],
    };

    let body = GeminiRequest::from_chat_request(&req);
    assert_eq!(body.contents[0].role.as_deref(), Some("model"));
}

#[test]
fn assistant_tool_calls_become_function_call_parts() {
    let req = ChatRequest {
        model: "gemini-1.5-flash".to_string(),
        tools: vec![],
        messages: vec![Message {
            role: Role::Assistant,
            content: None,
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "get_weather".to_string(),
                arguments: serde_json::json!({"city": "Paris"}),
                provider_metadata: None,
            }],
            tool_call_id: None,
        }],
    };

    let body = GeminiRequest::from_chat_request(&req);
    match &body.contents[0].parts[0] {
        GeminiPart::FunctionCall { function_call, .. } => {
            assert_eq!(function_call.name, "get_weather");
            assert_eq!(function_call.args, serde_json::json!({"city": "Paris"}));
        }
        other => panic!("expected FunctionCall part, got {other:?}"),
    }
}

#[test]
fn tool_call_thought_signature_round_trips_through_provider_metadata() {
    let raw = r#"{
        "candidates": [{
            "content": {"role": "model", "parts": [{"functionCall": {"name": "get_weather", "args": {}}, "thoughtSignature": "sig123"}]},
            "finishReason": "STOP"
        }]
    }"#;
    let parsed: GeminiResponse = serde_json::from_str(raw).unwrap();
    let response = parsed.candidates.into_iter().next().unwrap().into_chat_response();
    let tc = &response.message.tool_calls[0];
    assert_eq!(tc.provider_metadata.as_deref(), Some("sig123"));

    let req = ChatRequest {
        model: "gemini-3.6-flash".to_string(),
        tools: vec![],
        messages: vec![Message {
            role: Role::Assistant,
            content: None,
            tool_calls: vec![tc.clone()],
            tool_call_id: None,
        }],
    };
    let body = GeminiRequest::from_chat_request(&req);
    match &body.contents[0].parts[0] {
        GeminiPart::FunctionCall { thought_signature, .. } => {
            assert_eq!(thought_signature.as_deref(), Some("sig123"));
        }
        other => panic!("expected FunctionCall part, got {other:?}"),
    }
}

#[test]
fn tool_result_becomes_function_response_keyed_by_call_id() {
    let req = ChatRequest {
        model: "gemini-1.5-flash".to_string(),
        tools: vec![],
        messages: vec![Message {
            role: Role::Tool,
            content: Some("sunny".to_string()),
            tool_calls: vec![],
            tool_call_id: Some("call_1".to_string()),
        }],
    };

    let body = GeminiRequest::from_chat_request(&req);
    assert_eq!(body.contents[0].role.as_deref(), Some("user"));
    match &body.contents[0].parts[0] {
        GeminiPart::FunctionResponse { function_response } => {
            assert_eq!(function_response.name, "call_1");
            assert_eq!(function_response.response["content"], serde_json::json!("sunny"));
        }
        other => panic!("expected FunctionResponse part, got {other:?}"),
    }
}

#[test]
fn tool_definitions_become_function_declarations() {
    let req = ChatRequest {
        model: "gemini-1.5-flash".to_string(),
        tools: vec![ToolDefinition {
            name: "get_weather".to_string(),
            description: "gets weather".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }],
        messages: vec![],
    };

    let body = GeminiRequest::from_chat_request(&req);
    assert_eq!(body.tools.len(), 1);
    assert_eq!(body.tools[0].function_declarations[0].name, "get_weather");
}

#[test]
fn response_with_text_only_parses_into_chat_response() {
    let raw = r#"{
        "candidates": [{
            "content": {"role": "model", "parts": [{"text": "hi there"}]},
            "finishReason": "STOP"
        }]
    }"#;
    let parsed: GeminiResponse = serde_json::from_str(raw).unwrap();
    let response = parsed.candidates.into_iter().next().unwrap().into_chat_response();

    assert_eq!(response.message.content, Some("hi there".to_string()));
    assert!(response.message.tool_calls.is_empty());
    assert_eq!(response.finish_reason, FinishReason::Stop);
}

#[test]
fn response_with_function_call_reports_tool_calls_finish_reason() {
    let raw = r#"{
        "candidates": [{
            "content": {"role": "model", "parts": [{"functionCall": {"name": "get_weather", "args": {"city": "Paris"}}}]},
            "finishReason": "STOP"
        }]
    }"#;
    let parsed: GeminiResponse = serde_json::from_str(raw).unwrap();
    let response = parsed.candidates.into_iter().next().unwrap().into_chat_response();

    assert_eq!(response.finish_reason, FinishReason::ToolCalls);
    assert_eq!(response.message.tool_calls.len(), 1);
    assert_eq!(response.message.tool_calls[0].name, "get_weather");
    assert_eq!(response.message.tool_calls[0].arguments, serde_json::json!({"city": "Paris"}));
}

#[test]
fn max_tokens_finish_reason_maps_to_length() {
    let raw = r#"{
        "candidates": [{
            "content": {"role": "model", "parts": [{"text": "cut off"}]},
            "finishReason": "MAX_TOKENS"
        }]
    }"#;
    let parsed: GeminiResponse = serde_json::from_str(raw).unwrap();
    let response = parsed.candidates.into_iter().next().unwrap().into_chat_response();

    assert_eq!(response.finish_reason, FinishReason::Length);
}

#[test]
fn take_event_splits_on_double_newline_and_buffers_partial() {
    let mut buf = b"data: {\"a\":1}\n\ndata: partial".to_vec();
    let event = take_event(&mut buf).unwrap();
    assert_eq!(event, b"data: {\"a\":1}\n\n");
    assert_eq!(buf, b"data: partial");
    assert!(take_event(&mut buf).is_none());
}

#[test]
fn sse_event_parses_content_delta() {
    let event = b"data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"hi\"}]}}]}\n\n";
    let chunk = parse_event(event).unwrap().unwrap();
    assert_eq!(chunk.delta, Some("hi".to_string()));
    assert_eq!(chunk.finish_reason, None);
}

#[test]
fn sse_event_reports_finish_reason() {
    let event = b"data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[]},\"finishReason\":\"STOP\"}]}\n\n";
    let chunk = parse_event(event).unwrap().unwrap();
    assert_eq!(chunk.finish_reason, Some(FinishReason::Stop));
}

#[test]
fn sse_event_with_blank_data_yields_no_item() {
    let event = b"\n\n";
    assert!(parse_event(event).is_none());
}
