use std::env;

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

const DEFAULT_BASE_URL: &str = "http://localhost:11434";
pub const DEFAULT_MODEL: &str = "llama3.1";

pub struct OllamaProvider {
    client: Client,
    base_url: String,
    model: String,
}

impl OllamaProvider {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: DEFAULT_BASE_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
        }
    }

    pub fn from_env() -> Self {
        let base_url = env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        Self {
            client: Client::new(),
            base_url,
            model: DEFAULT_MODEL.to_string(),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn endpoint(&self) -> String {
        format!("{}/api/chat", self.base_url)
    }

    async fn map_error(resp: reqwest::Response) -> ProviderError {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderError::Auth,
            StatusCode::TOO_MANY_REQUESTS => ProviderError::RateLimited { retry_after: None },
            _ => ProviderError::Other(format!("ollama api error ({status}): {body}")),
        }
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = OllamaRequest::from_chat_request(&req, false);

        let resp = self.client.post(self.endpoint()).json(&body).send().await?;

        if !resp.status().is_success() {
            return Err(Self::map_error(resp).await);
        }

        let parsed: OllamaChatChunk = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        Ok(parsed.into_chat_response())
    }

    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk, ProviderError>>, ProviderError> {
        let body = OllamaRequest::from_chat_request(&req, true);

        let resp = self.client.post(self.endpoint()).json(&body).send().await?;

        if !resp.status().is_success() {
            return Err(Self::map_error(resp).await);
        }

        Ok(Box::pin(ndjson_stream(resp)))
    }
}

// -- NDJSON parsing -------------------------------------------------------

/// Pull one complete line (up to '\n') out of the buffer, if present.
fn take_line(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    let pos = buf.iter().position(|&b| b == b'\n')?;
    let line: Vec<u8> = buf.drain(..=pos).collect();
    Some(line)
}

fn parse_line(line: &[u8]) -> Option<Result<StreamChunk, ProviderError>> {
    let text = String::from_utf8_lossy(line);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed: Result<OllamaChatChunk, _> = serde_json::from_str(trimmed);
    Some(match parsed {
        Ok(chunk) => Ok(chunk.into_stream_chunk()),
        Err(e) => Err(ProviderError::Parse(e.to_string())),
    })
}

fn ndjson_stream(resp: reqwest::Response) -> impl futures_core::Stream<Item = Result<StreamChunk, ProviderError>> {
    stream! {
        let mut inner = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();

        loop {
            if let Some(line) = take_line(&mut buf) {
                if let Some(item) = parse_line(&line) {
                    yield item;
                }
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
                None => {
                    if let Some(item) = parse_line(&buf) {
                        yield item;
                    }
                    break;
                }
            }
        }
    }
}

// -- Ollama wire format -----------------------------------------------------

#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OllamaTool>,
    stream: bool,
}

impl OllamaRequest {
    fn from_chat_request(req: &ChatRequest, stream: bool) -> Self {
        Self {
            model: req.model.clone(),
            messages: req.messages.iter().map(OllamaMessage::from_message).collect(),
            tools: req.tools.iter().map(OllamaTool::from_definition).collect(),
            stream,
        }
    }
}

#[derive(Serialize)]
struct OllamaMessage {
    role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<OllamaToolCall>,
}

impl OllamaMessage {
    fn from_message(msg: &Message) -> Self {
        Self {
            role: msg.role,
            content: msg.content.clone(),
            tool_calls: msg.tool_calls.iter().map(OllamaToolCall::from_tool_call).collect(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct OllamaToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    function: OllamaFunctionCall,
}

impl OllamaToolCall {
    fn from_tool_call(tc: &ToolCall) -> Self {
        Self {
            id: Some(tc.id.clone()),
            function: OllamaFunctionCall {
                name: tc.name.clone(),
                arguments: tc.arguments.clone(),
            },
        }
    }

    fn into_tool_call(self) -> ToolCall {
        ToolCall {
            id: self.id.unwrap_or_default(),
            name: self.function.name,
            arguments: self.function.arguments,
            provider_metadata: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct OllamaFunctionCall {
    name: String,
    arguments: serde_json::Value,
}

#[derive(Serialize)]
struct OllamaTool {
    #[serde(rename = "type")]
    kind: String,
    function: OllamaFunctionDef,
}

impl OllamaTool {
    fn from_definition(def: &ToolDefinition) -> Self {
        Self {
            kind: "function".to_string(),
            function: OllamaFunctionDef {
                name: def.name.clone(),
                description: def.description.clone(),
                parameters: def.parameters.clone(),
            },
        }
    }
}

#[derive(Serialize)]
struct OllamaFunctionDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct OllamaChatChunk {
    message: Option<OllamaResponseMessage>,
    #[serde(default)]
    done: bool,
    done_reason: Option<String>,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    role: Role,
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OllamaToolCall>,
}

impl OllamaChatChunk {
    fn finish_reason(&self) -> Option<FinishReason> {
        if !self.done {
            return None;
        }
        let has_tool_calls = self
            .message
            .as_ref()
            .map(|m| !m.tool_calls.is_empty())
            .unwrap_or(false);
        if has_tool_calls {
            return Some(FinishReason::ToolCalls);
        }
        Some(match self.done_reason.as_deref() {
            Some("length") => FinishReason::Length,
            _ => FinishReason::Stop,
        })
    }

    fn into_chat_response(self) -> ChatResponse {
        let finish_reason = self.finish_reason().unwrap_or(FinishReason::Stop);
        let message = match self.message {
            Some(m) => Message {
                role: m.role,
                content: m.content,
                tool_calls: m.tool_calls.into_iter().map(OllamaToolCall::into_tool_call).collect(),
                tool_call_id: None,
            },
            None => Message {
                role: Role::Assistant,
                content: None,
                tool_calls: vec![],
                tool_call_id: None,
            },
        };
        ChatResponse { message, finish_reason }
    }

    fn into_stream_chunk(self) -> StreamChunk {
        let finish_reason = self.finish_reason();
        let (delta, tool_call_delta) = match self.message {
            Some(m) => (
                m.content,
                m.tool_calls.into_iter().next().map(OllamaToolCall::into_tool_call),
            ),
            None => (None, None),
        };
        StreamChunk {
            delta,
            tool_call_delta,
            finish_reason,
        }
    }
}

#[cfg(test)]
#[path = "ollama_test.rs"]
mod tests;
