use std::env;
use std::time::Duration;

use async_stream::stream;
use async_trait::async_trait;
use futures_core::stream::BoxStream;
use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};

use crate::traits::{
    ChatRequest, ChatResponse, FinishReason, LlmProvider, Message, ProviderError, Role,
    StreamChunk, ToolCall, ToolDefinition,
};

const DEFAULT_BASE_URL: &str = "https://api.groq.com/openai/v1";
pub const DEFAULT_MODEL: &str = "openai/gpt-oss-20b";

pub struct GroqProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl GroqProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
        }
    }

    pub fn from_env() -> Result<Self, ProviderError> {
        let api_key = env::var("GROQ_API_KEY")
            .map_err(|_| ProviderError::Auth)?;
        Ok(Self::new(api_key))
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    async fn map_error(resp: reqwest::Response) -> ProviderError {
        let status = resp.status();
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs);
        let body = resp.text().await.unwrap_or_default();

        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderError::Auth,
            StatusCode::TOO_MANY_REQUESTS => ProviderError::RateLimited { retry_after },
            _ => ProviderError::Other(format!("groq api error ({status}): {body}")),
        }
    }
}

#[async_trait]
impl LlmProvider for GroqProvider {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = GroqRequest::from_chat_request(&req, false);

        let resp = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(Self::map_error(resp).await);
        }

        let parsed: GroqResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::Parse("no choices in response".to_string()))?;

        Ok(ChatResponse {
            message: choice.message.into_message(),
            finish_reason: choice.finish_reason.into(),
        })
    }

    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk, ProviderError>>, ProviderError> {
        let body = GroqRequest::from_chat_request(&req, true);

        let resp = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(Self::map_error(resp).await);
        }

        Ok(Box::pin(sse_stream(resp)))
    }
}

// -- SSE parsing --------------------------------------------------------

/// Pull one complete SSE event ("...\n\n") out of the buffer, if present.
fn take_event(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    let needle = b"\n\n";
    let pos = buf.windows(needle.len()).position(|w| w == needle)?;
    let event: Vec<u8> = buf.drain(..pos + needle.len()).collect();
    Some(event)
}

fn parse_event(event: &[u8]) -> Option<Result<StreamChunk, ProviderError>> {
    let text = String::from_utf8_lossy(event);
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:"))
        else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            return None;
        }
        let parsed: Result<GroqStreamChunk, _> = serde_json::from_str(data);
        return Some(match parsed {
            Ok(chunk) => Ok(chunk.into_stream_chunk()),
            Err(e) => Err(ProviderError::Parse(e.to_string())),
        });
    }
    None
}

fn sse_stream(resp: reqwest::Response) -> impl futures_core::Stream<Item = Result<StreamChunk, ProviderError>> {
    stream! {
        let mut inner = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();

        loop {
            if let Some(event) = take_event(&mut buf) {
                if let Some(item) = parse_event(&event) {
                    yield item;
                }
                // blank/[DONE] event, keep looking for the next one
                continue;
            }

            match inner.next().await {
                Some(Ok(bytes)) => {
                    buf.extend_from_slice(&bytes);
                }
                Some(Err(e)) => {
                    yield Err(ProviderError::Network(e));
                    break;
                }
                None => break,
            }
        }
    }
}

// -- OpenAI-compatible wire format --------------------------------------

#[derive(Serialize)]
struct GroqRequest {
    model: String,
    messages: Vec<GroqMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<GroqTool>,
    stream: bool,
}

impl GroqRequest {
    fn from_chat_request(req: &ChatRequest, stream: bool) -> Self {
        Self {
            model: req.model.clone(),
            messages: req.messages.iter().map(GroqMessage::from_message).collect(),
            tools: req.tools.iter().map(GroqTool::from_definition).collect(),
            stream,
        }
    }
}

#[derive(Serialize)]
struct GroqMessage {
    role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<GroqToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

impl GroqMessage {
    fn from_message(msg: &Message) -> Self {
        Self {
            role: msg.role,
            content: msg.content.clone(),
            tool_calls: msg.tool_calls.iter().map(GroqToolCall::from_tool_call).collect(),
            tool_call_id: msg.tool_call_id.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct GroqToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: GroqFunctionCall,
}

impl GroqToolCall {
    fn from_tool_call(tc: &ToolCall) -> Self {
        Self {
            id: tc.id.clone(),
            kind: "function".to_string(),
            function: GroqFunctionCall {
                name: tc.name.clone(),
                arguments: tc.arguments.to_string(),
            },
        }
    }

    fn into_tool_call(self) -> ToolCall {
        let arguments = serde_json::from_str(&self.function.arguments)
            .unwrap_or(serde_json::Value::String(self.function.arguments));
        ToolCall {
            id: self.id,
            name: self.function.name,
            arguments,
            provider_metadata: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct GroqFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct GroqTool {
    #[serde(rename = "type")]
    kind: String,
    function: GroqFunctionDef,
}

impl GroqTool {
    fn from_definition(def: &ToolDefinition) -> Self {
        Self {
            kind: "function".to_string(),
            function: GroqFunctionDef {
                name: def.name.clone(),
                description: def.description.clone(),
                parameters: def.parameters.clone(),
            },
        }
    }
}

#[derive(Serialize)]
struct GroqFunctionDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct GroqResponse {
    choices: Vec<GroqChoice>,
}

#[derive(Deserialize)]
struct GroqChoice {
    message: GroqResponseMessage,
    finish_reason: GroqFinishReason,
}

#[derive(Deserialize)]
struct GroqResponseMessage {
    role: Role,
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<GroqToolCall>,
}

impl GroqResponseMessage {
    fn into_message(self) -> Message {
        Message {
            role: self.role,
            content: self.content,
            tool_calls: self.tool_calls.into_iter().map(GroqToolCall::into_tool_call).collect(),
            tool_call_id: None,
        }
    }
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum GroqFinishReason {
    Stop,
    ToolCalls,
    Length,
    #[serde(other)]
    Other,
}

impl From<GroqFinishReason> for FinishReason {
    fn from(r: GroqFinishReason) -> Self {
        match r {
            GroqFinishReason::Stop => FinishReason::Stop,
            GroqFinishReason::ToolCalls => FinishReason::ToolCalls,
            GroqFinishReason::Length | GroqFinishReason::Other => FinishReason::Length,
        }
    }
}

#[derive(Deserialize)]
struct GroqStreamChunk {
    choices: Vec<GroqStreamChoice>,
}

#[derive(Deserialize)]
struct GroqStreamChoice {
    delta: GroqStreamDelta,
    finish_reason: Option<GroqFinishReason>,
}

#[derive(Deserialize, Default)]
struct GroqStreamDelta {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<GroqToolCallDelta>,
}

#[derive(Deserialize)]
struct GroqToolCallDelta {
    id: Option<String>,
    function: GroqFunctionCallDelta,
}

#[derive(Deserialize, Default)]
struct GroqFunctionCallDelta {
    name: Option<String>,
    #[serde(default)]
    arguments: String,
}

impl GroqStreamChunk {
    fn into_stream_chunk(mut self) -> StreamChunk {
        let choice = if self.choices.is_empty() {
            None
        } else {
            Some(self.choices.remove(0))
        };

        let Some(choice) = choice else {
            return StreamChunk {
                delta: None,
                tool_call_delta: None,
                finish_reason: None,
            };
        };

        let tool_call_delta = choice.delta.tool_calls.into_iter().next().map(|tc| ToolCall {
            id: tc.id.unwrap_or_default(),
            name: tc.function.name.unwrap_or_default(),
            arguments: serde_json::from_str(&tc.function.arguments)
                .unwrap_or(serde_json::Value::String(tc.function.arguments)),
            provider_metadata: None,
        });

        StreamChunk {
            delta: choice.delta.content,
            tool_call_delta,
            finish_reason: choice.finish_reason.map(Into::into),
        }
    }
}

#[cfg(test)]
#[path = "groq_test.rs"]
mod tests;
