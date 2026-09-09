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

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
pub const DEFAULT_MODEL: &str = "gemini-3.6-flash";

pub struct GeminiProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl GeminiProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
        }
    }

    pub fn from_env() -> Result<Self, ProviderError> {
        let api_key = env::var("GEMINI_API_KEY").map_err(|_| ProviderError::Auth)?;
        Ok(Self::new(api_key))
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    fn endpoint(&self, model: &str, stream: bool) -> String {
        let method = if stream {
            "streamGenerateContent?alt=sse"
        } else {
            "generateContent"
        };
        format!("{}/models/{}:{}", self.base_url, model, method)
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
            _ => ProviderError::Other(format!("gemini api error ({status}): {body}")),
        }
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let model = req.model.clone();
        let body = GeminiRequest::from_chat_request(&req);

        let resp = self
            .client
            .post(self.endpoint(&model, false))
            .header("x-goog-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(Self::map_error(resp).await);
        }

        let parsed: GeminiResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        let candidate = parsed
            .candidates
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::Parse("no candidates in response".to_string()))?;

        Ok(candidate.into_chat_response())
    }

    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk, ProviderError>>, ProviderError> {
        let model = req.model.clone();
        let body = GeminiRequest::from_chat_request(&req);

        let resp = self
            .client
            .post(self.endpoint(&model, true))
            .header("x-goog-api-key", &self.api_key)
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

fn take_event(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    let needle = b"\n\n";
    let pos = buf.windows(needle.len()).position(|w| w == needle)?;
    let event: Vec<u8> = buf.drain(..pos + needle.len()).collect();
    Some(event)
}

fn parse_event(event: &[u8]) -> Option<Result<StreamChunk, ProviderError>> {
    let text = String::from_utf8_lossy(event);
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:")) else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        let parsed: Result<GeminiResponse, _> = serde_json::from_str(data);
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

// -- Gemini wire format ---------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<GeminiTool>,
}

impl GeminiRequest {
    fn from_chat_request(req: &ChatRequest) -> Self {
        let mut system_parts: Vec<GeminiPart> = Vec::new();
        let mut contents: Vec<GeminiContent> = Vec::new();

        for msg in &req.messages {
            match msg.role {
                Role::System => {
                    if let Some(text) = &msg.content {
                        system_parts.push(GeminiPart::text(text.clone()));
                    }
                }
                _ => contents.push(GeminiContent::from_message(msg)),
            }
        }

        let system_instruction = if system_parts.is_empty() {
            None
        } else {
            Some(GeminiContent {
                role: None,
                parts: system_parts,
            })
        };

        let tools = if req.tools.is_empty() {
            Vec::new()
        } else {
            vec![GeminiTool {
                function_declarations: req.tools.iter().map(GeminiFunctionDecl::from_definition).collect(),
            }]
        };

        Self {
            contents,
            system_instruction,
            tools,
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

impl GeminiContent {
    fn from_message(msg: &Message) -> Self {
        let role = match msg.role {
            Role::User | Role::Tool => "user",
            Role::Assistant => "model",
            Role::System => unreachable!("system messages are routed to system_instruction"),
        };

        let mut parts = Vec::new();

        if msg.role == Role::Tool {
            // Message carries only `tool_call_id`, not the originating function name that
            // Gemini's functionResponse requires — fall back to the call id as the name.
            if let Some(id) = &msg.tool_call_id {
                parts.push(GeminiPart::FunctionResponse {
                    function_response: GeminiFunctionResponse {
                        name: id.clone(),
                        response: serde_json::json!({ "content": msg.content }),
                    },
                });
            }
        } else {
            if let Some(text) = &msg.content {
                parts.push(GeminiPart::text(text.clone()));
            }
            for tc in &msg.tool_calls {
                parts.push(GeminiPart::FunctionCall {
                    function_call: GeminiFunctionCall {
                        name: tc.name.clone(),
                        args: tc.arguments.clone(),
                    },
                    thought_signature: tc.provider_metadata.clone(),
                });
            }
        }

        Self {
            role: Some(role.to_string()),
            parts,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
enum GeminiPart {
    Text { text: String },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: GeminiFunctionCall,
        #[serde(rename = "thoughtSignature", skip_serializing_if = "Option::is_none", default)]
        thought_signature: Option<String>,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: GeminiFunctionResponse,
    },
}

impl GeminiPart {
    fn text(text: String) -> Self {
        GeminiPart::Text { text }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct GeminiFunctionCall {
    name: String,
    args: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug)]
struct GeminiFunctionResponse {
    name: String,
    response: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiTool {
    function_declarations: Vec<GeminiFunctionDecl>,
}

#[derive(Serialize)]
struct GeminiFunctionDecl {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

impl GeminiFunctionDecl {
    fn from_definition(def: &ToolDefinition) -> Self {
        Self {
            name: def.name.clone(),
            description: def.description.clone(),
            parameters: def.parameters.clone(),
        }
    }
}

#[derive(Deserialize)]
struct GeminiResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    #[serde(default)]
    content: GeminiContent,
    finish_reason: Option<GeminiFinishReason>,
}

impl GeminiCandidate {
    fn into_parts(self) -> (Option<String>, Vec<ToolCall>) {
        let mut text = String::new();
        let mut tool_calls = Vec::new();

        for part in self.content.parts {
            match part {
                GeminiPart::Text { text: t } => text.push_str(&t),
                GeminiPart::FunctionCall { function_call, thought_signature } => {
                    tool_calls.push(ToolCall {
                        id: function_call.name.clone(),
                        name: function_call.name,
                        arguments: function_call.args,
                        provider_metadata: thought_signature,
                    });
                }
                GeminiPart::FunctionResponse { .. } => {}
            }
        }

        (if text.is_empty() { None } else { Some(text) }, tool_calls)
    }

    fn into_chat_response(self) -> ChatResponse {
        let finish_reason_raw = self.finish_reason;
        let (content, tool_calls) = self.into_parts();

        let finish_reason = if !tool_calls.is_empty() {
            FinishReason::ToolCalls
        } else {
            finish_reason_raw.map(Into::into).unwrap_or(FinishReason::Stop)
        };

        ChatResponse {
            message: Message {
                role: Role::Assistant,
                content,
                tool_calls,
                tool_call_id: None,
            },
            finish_reason,
        }
    }

    fn into_stream_chunk(self) -> StreamChunk {
        let finish_reason_raw = self.finish_reason;
        let (delta, mut tool_calls) = self.into_parts();

        let finish_reason = if !tool_calls.is_empty() {
            Some(FinishReason::ToolCalls)
        } else {
            finish_reason_raw.map(Into::into)
        };

        StreamChunk {
            delta,
            tool_call_delta: if tool_calls.is_empty() { None } else { Some(tool_calls.remove(0)) },
            finish_reason,
        }
    }
}

impl GeminiResponse {
    fn into_stream_chunk(mut self) -> StreamChunk {
        if self.candidates.is_empty() {
            return StreamChunk {
                delta: None,
                tool_call_delta: None,
                finish_reason: None,
            };
        }
        self.candidates.remove(0).into_stream_chunk()
    }
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum GeminiFinishReason {
    Stop,
    MaxTokens,
    Safety,
    Recitation,
    #[serde(other)]
    Other,
}

impl From<GeminiFinishReason> for FinishReason {
    fn from(r: GeminiFinishReason) -> Self {
        match r {
            GeminiFinishReason::Stop => FinishReason::Stop,
            GeminiFinishReason::MaxTokens => FinishReason::Length,
            GeminiFinishReason::Safety | GeminiFinishReason::Recitation | GeminiFinishReason::Other => {
                FinishReason::Stop
            }
        }
    }
}

#[cfg(test)]
#[path = "gemini_test.rs"]
mod tests;
