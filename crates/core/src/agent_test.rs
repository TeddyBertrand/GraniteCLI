use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_core::stream::BoxStream;
use provider::{
    ChatRequest, ChatResponse, FinishReason, LlmProvider, Message, ProviderError, Role,
    StreamChunk, ToolCall,
};
use serde_json::json;
use tools::{Tool, ToolError, ToolOutput};

use super::*;

struct ScriptedProvider {
    responses: Mutex<Vec<Result<ChatResponse, ProviderError>>>,
    calls: AtomicUsize,
}

impl ScriptedProvider {
    fn new(responses: Vec<Result<ChatResponse, ProviderError>>) -> Self {
        Self {
            responses: Mutex::new(responses),
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut responses = self.responses.lock().unwrap();
        assert!(!responses.is_empty(), "provider called more times than scripted");
        responses.remove(0)
    }

    async fn chat_stream(
        &self,
        _req: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk, ProviderError>>, ProviderError> {
        unimplemented!("not used by agent tests")
    }
}

fn assistant_text(text: &str) -> ChatResponse {
    ChatResponse {
        message: Message {
            role: Role::Assistant,
            content: Some(text.to_string()),
            tool_calls: vec![],
            tool_call_id: None,
        },
        finish_reason: FinishReason::Stop,
    }
}

fn assistant_tool_call(id: &str, name: &str, args: serde_json::Value) -> ChatResponse {
    ChatResponse {
        message: Message {
            role: Role::Assistant,
            content: None,
            tool_calls: vec![ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments: args,
            }],
            tool_call_id: None,
        },
        finish_reason: FinishReason::ToolCalls,
    }
}

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "echoes the 'text' argument back"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({"type": "object", "properties": {"text": {"type": "string"}}})
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'text'".to_string()))?;
        Ok(ToolOutput {
            content: format!("echoed: {text}"),
        })
    }
}

#[tokio::test]
async fn single_turn_no_tool_call_returns_content() {
    let provider = Arc::new(ScriptedProvider::new(vec![Ok(assistant_text("hi there"))]));
    let mut agent = Agent::new(provider, "test-model");

    let answer = agent.run("hello".to_string()).await.unwrap();

    assert_eq!(answer, "hi there");
}

#[tokio::test]
async fn run_without_provider_returns_no_provider_error() {
    let mut agent = Agent::without_provider("test-model");

    let err = agent.run("hello".to_string()).await.unwrap_err();

    assert!(matches!(err, AgentError::NoProvider));
    assert!(!agent.has_provider());
}

#[tokio::test]
async fn set_provider_lets_run_succeed_afterwards() {
    let provider = Arc::new(ScriptedProvider::new(vec![Ok(assistant_text("hi there"))]));
    let mut agent = Agent::without_provider("test-model");
    assert!(!agent.has_provider());

    agent.set_provider(provider);
    assert!(agent.has_provider());

    let answer = agent.run("hello".to_string()).await.unwrap();
    assert_eq!(answer, "hi there");
}

#[tokio::test]
async fn tool_call_round_trip_then_final_answer() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        Ok(assistant_tool_call("call_1", "echo", json!({"text": "ping"}))),
        Ok(assistant_text("done")),
    ]));
    let mut agent = Agent::new(provider, "test-model").with_tool(Arc::new(EchoTool));

    let answer = agent.run("go".to_string()).await.unwrap();

    assert_eq!(answer, "done");
    let messages = agent.context().messages();
    let tool_msg = messages
        .iter()
        .find(|m| m.role == Role::Tool)
        .expect("tool result message pushed to context");
    assert_eq!(tool_msg.content.as_deref(), Some("echoed: ping"));
    assert_eq!(tool_msg.tool_call_id.as_deref(), Some("call_1"));
}

#[tokio::test]
async fn unknown_tool_call_reports_error_in_context_and_continues() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        Ok(assistant_tool_call("call_1", "nope", json!({}))),
        Ok(assistant_text("recovered")),
    ]));
    let mut agent = Agent::new(provider, "test-model");

    let answer = agent.run("go".to_string()).await.unwrap();

    assert_eq!(answer, "recovered");
}

#[tokio::test]
async fn rate_limited_retries_then_succeeds() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        Err(ProviderError::RateLimited {
            retry_after: Some(std::time::Duration::from_millis(1)),
        }),
        Ok(assistant_text("ok")),
    ]));
    let mut agent = Agent::new(provider, "test-model");

    let answer = agent.run("go".to_string()).await.unwrap();

    assert_eq!(answer, "ok");
}

#[tokio::test]
async fn auth_error_aborts_immediately() {
    let provider = Arc::new(ScriptedProvider::new(vec![Err(ProviderError::Auth)]));
    let mut agent = Agent::new(provider, "test-model");

    let err = agent.run("go".to_string()).await.unwrap_err();

    assert!(matches!(err, AgentError::Provider(ProviderError::Auth)));
}

struct FixedConfirmHook(bool);

#[async_trait]
impl ConfirmHook for FixedConfirmHook {
    async fn confirm(&self, _request: ConfirmRequest) -> bool {
        self.0
    }
}

#[tokio::test]
async fn confirm_hook_denial_short_circuits_tool_execution() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        Ok(assistant_tool_call("call_1", "echo", json!({"text": "ping"}))),
        Ok(assistant_text("done")),
    ]));
    let mut agent = Agent::new(provider, "test-model")
        .with_tool(Arc::new(EchoTool))
        .with_confirm_hook(Arc::new(FixedConfirmHook(false)));

    let answer = agent.run("go".to_string()).await.unwrap();

    assert_eq!(answer, "done");
    let messages = agent.context().messages();
    let tool_msg = messages
        .iter()
        .find(|m| m.role == Role::Tool)
        .expect("tool result message pushed to context");
    assert!(tool_msg.content.as_deref().unwrap().contains("rejected by user"));
}

#[tokio::test]
async fn confirm_hook_approval_lets_tool_execute() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        Ok(assistant_tool_call("call_1", "echo", json!({"text": "ping"}))),
        Ok(assistant_text("done")),
    ]));
    let mut agent = Agent::new(provider, "test-model")
        .with_tool(Arc::new(EchoTool))
        .with_confirm_hook(Arc::new(FixedConfirmHook(true)));

    agent.run("go".to_string()).await.unwrap();

    let messages = agent.context().messages();
    let tool_msg = messages
        .iter()
        .find(|m| m.role == Role::Tool)
        .expect("tool result message pushed to context");
    assert_eq!(tool_msg.content.as_deref(), Some("echoed: ping"));
}

#[tokio::test]
async fn no_confirm_hook_is_fail_open() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        Ok(assistant_tool_call("call_1", "echo", json!({"text": "ping"}))),
        Ok(assistant_text("done")),
    ]));
    let mut agent = Agent::new(provider, "test-model").with_tool(Arc::new(EchoTool));

    agent.run("go".to_string()).await.unwrap();

    let messages = agent.context().messages();
    let tool_msg = messages
        .iter()
        .find(|m| m.role == Role::Tool)
        .expect("tool result message pushed to context");
    assert_eq!(tool_msg.content.as_deref(), Some("echoed: ping"));
}

#[tokio::test]
async fn max_iterations_reached_when_tool_calls_never_stop() {
    let responses = (0..3)
        .map(|_| Ok(assistant_tool_call("call_x", "echo", json!({"text": "x"}))))
        .collect();
    let provider = Arc::new(ScriptedProvider::new(responses));
    let mut agent = Agent::new(provider, "test-model")
        .with_tool(Arc::new(EchoTool))
        .with_max_iterations(3);

    let err = agent.run("go".to_string()).await.unwrap_err();

    assert!(matches!(err, AgentError::MaxIterationsReached(3)));
}
