use std::collections::HashMap;
use std::sync::Arc;

use futures_util::future::join_all;
use provider::{ChatRequest, FinishReason, LlmProvider, Message, ProviderError, Role, ToolDefinition};
use thiserror::Error;
use tools::{Tool, ToolError};

use crate::confirm::{ConfirmHook, ConfirmRequest};
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

    #[error("no API key configured for this provider")]
    NoProvider,
}

pub struct Agent {
    provider: Option<Arc<dyn LlmProvider>>,
    tools: HashMap<String, Arc<dyn Tool>>,
    context: ConversationContext,
    model: String,
    max_iterations: usize,
    confirm_hook: Option<Arc<dyn ConfirmHook>>,
}

impl Agent {
    pub fn new(provider: Arc<dyn LlmProvider>, model: impl Into<String>) -> Self {
        Self {
            provider: Some(provider),
            tools: HashMap::new(),
            context: ConversationContext::new(),
            model: model.into(),
            max_iterations: 10,
            confirm_hook: None,
        }
    }

    /// Builds an agent with no provider set yet — usable once
    /// [`Agent::set_provider`] is called.
    pub fn without_provider(model: impl Into<String>) -> Self {
        Self {
            provider: None,
            tools: HashMap::new(),
            context: ConversationContext::new(),
            model: model.into(),
            max_iterations: 10,
            confirm_hook: None,
        }
    }

    /// Sets (or replaces) the provider used for subsequent turns, e.g. after
    /// the user supplies an API key mid-session.
    pub fn set_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        self.provider = Some(provider);
    }

    pub fn has_provider(&self) -> bool {
        self.provider.is_some()
    }

    pub fn with_tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.insert(tool.name().to_string(), tool);
        self
    }

    pub fn set_model(&mut self, model: impl Into<String>) {
        self.model = model.into();
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn with_max_iterations(mut self, max_iterations: usize) -> Self {
        self.max_iterations = max_iterations;
        self
    }

    pub fn with_confirm_hook(mut self, hook: Arc<dyn ConfirmHook>) -> Self {
        self.confirm_hook = Some(hook);
        self
    }

    pub fn set_confirm_hook(&mut self, hook: Arc<dyn ConfirmHook>) {
        self.confirm_hook = Some(hook);
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
        let provider = self.provider.as_ref().ok_or(AgentError::NoProvider)?;
        let mut attempts = 0;
        loop {
            match provider.chat(req.clone()).await {
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

    async fn execute_tool_call(&self, call: &provider::ToolCall, tool: &Arc<dyn Tool>) -> Message {
        let result = tool.execute(call.arguments.clone()).await;

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

    fn denied_message(call: &provider::ToolCall, reason: &str) -> Message {
        Message {
            role: Role::Tool,
            content: Some(format!("error: {}", ToolError::Denied(reason.to_string()))),
            tool_calls: vec![],
            tool_call_id: Some(call.id.clone()),
        }
    }

    pub async fn run(&mut self, user_input: String) -> Result<String, AgentError> {
        if self.provider.is_none() {
            return Err(AgentError::NoProvider);
        }

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

            let mut approved: Vec<(&provider::ToolCall, Arc<dyn Tool>)> = Vec::new();
            let mut resolved_msgs: Vec<Message> = Vec::new();

            for call in &message.tool_calls {
                let Some(tool) = self.tools.get(&call.name).cloned() else {
                    resolved_msgs.push(Message {
                        role: Role::Tool,
                        content: Some(format!("error: {}", ToolError::NotFound(call.name.clone()))),
                        tool_calls: vec![],
                        tool_call_id: Some(call.id.clone()),
                    });
                    continue;
                };

                let allowed = match &self.confirm_hook {
                    Some(hook) => {
                        let request = ConfirmRequest {
                            tool_name: tool.name().to_string(),
                            risk: tool.risk(&call.arguments),
                            detail: tool.describe(&call.arguments),
                        };
                        hook.confirm(request).await
                    }
                    None => true,
                };

                if allowed {
                    approved.push((call, tool));
                } else {
                    resolved_msgs.push(Self::denied_message(call, "rejected by user"));
                }
            }

            let executed = join_all(approved.iter().map(|(call, tool)| self.execute_tool_call(call, tool))).await;
            resolved_msgs.extend(executed);

            for tool_msg in resolved_msgs {
                self.context.push(tool_msg);
            }
        }

        Err(AgentError::MaxIterationsReached(self.max_iterations))
    }
}

#[cfg(test)]
#[path = "agent_test.rs"]
mod tests;
