use std::collections::HashMap;
use std::sync::Arc;

use futures_util::future::join_all;
use provider::{ChatRequest, FinishReason, LlmProvider, Message, ProviderError, Role, ToolDefinition};
use thiserror::Error;
use tools::{Tool, ToolError};

use crate::context::ConversationContext;

const MAX_PROVIDER_RETRIES: usize = 3;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),

    #[error("tool error: {0}")]
    Tool(#[from] ToolError),

    #[error("max iterations ({0}) reached without a final answer")]
    MaxIterationsReached(usize),
}

pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    tools: HashMap<String, Arc<dyn Tool>>,
    context: ConversationContext,
    model: String,
    max_iterations: usize,
}

impl Agent {
    pub fn new(provider: Arc<dyn LlmProvider>, model: impl Into<String>) -> Self {
        Self {
            provider,
            tools: HashMap::new(),
            context: ConversationContext::new(),
            model: model.into(),
            max_iterations: 10,
        }
    }

    pub fn with_tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.insert(tool.name().to_string(), tool);
        self
    }

    pub fn with_max_iterations(mut self, max_iterations: usize) -> Self {
        self.max_iterations = max_iterations;
        self
    }

    pub fn context(&self) -> &ConversationContext {
        &self.context
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters(),
            })
            .collect()
    }

    async fn chat_with_retry(&self, req: ChatRequest) -> Result<provider::ChatResponse, AgentError> {
        let mut attempts = 0;
        loop {
            match self.provider.chat(req.clone()).await {
                Ok(resp) => return Ok(resp),
                Err(ProviderError::Auth) => return Err(AgentError::Provider(ProviderError::Auth)),
                Err(err @ ProviderError::RateLimited { retry_after }) => {
                    attempts += 1;
                    if attempts > MAX_PROVIDER_RETRIES {
                        return Err(AgentError::Provider(err));
                    }
                    let delay = retry_after.unwrap_or(std::time::Duration::from_secs(1));
                    tokio::time::sleep(delay).await;
                }
                Err(err @ (ProviderError::Network(_) | ProviderError::Parse(_) | ProviderError::Other(_))) => {
                    attempts += 1;
                    if attempts > MAX_PROVIDER_RETRIES {
                        return Err(AgentError::Provider(err));
                    }
                    let delay = std::time::Duration::from_millis(500) * attempts as u32;
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    async fn dispatch_tool_call(&self, call: &provider::ToolCall) -> Message {
        let result = match self.tools.get(&call.name) {
            Some(tool) => tool.execute(call.arguments.clone()).await,
            None => Err(ToolError::NotFound(call.name.clone())),
        };

        let content = match result {
            Ok(output) => output.content,
            Err(err) => format!("error: {err}"),
        };

        Message {
            role: Role::Tool,
            content: Some(content),
            tool_calls: vec![],
            tool_call_id: Some(call.id.clone()),
        }
    }

    pub async fn run(&mut self, user_input: String) -> Result<String, AgentError> {
        self.context.push(Message {
            role: Role::User,
            content: Some(user_input),
            tool_calls: vec![],
            tool_call_id: None,
        });

        for i in 0..self.max_iterations {
            let req = ChatRequest {
                messages: self.context.messages().to_vec(),
                tools: self.tool_definitions(),
                model: self.model.clone(),
            };

            let resp = self.chat_with_retry(req).await?;
            let message = resp.message;
            self.context.push(message.clone());

            if resp.finish_reason != FinishReason::ToolCalls {
                return Ok(message.content.unwrap_or_default());
            }

            if i + 1 == self.max_iterations {
                // model asked for tool calls but we're stopping now — respond to each
                // so no tool call is left dangling in history for future requests.
                for call in &message.tool_calls {
                    self.context.push(Message {
                        role: Role::Tool,
                        content: Some("error: max iterations reached before tool could run".to_string()),
                        tool_calls: vec![],
                        tool_call_id: Some(call.id.clone()),
                    });
                }
                break;
            }

            let tool_msgs = join_all(message.tool_calls.iter().map(|call| self.dispatch_tool_call(call))).await;
            for tool_msg in tool_msgs {
                self.context.push(tool_msg);
            }
        }

        Err(AgentError::MaxIterationsReached(self.max_iterations))
    }
}

#[cfg(test)]
#[path = "agent_test.rs"]
mod tests;
