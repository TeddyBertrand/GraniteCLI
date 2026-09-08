use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: String,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("tool not found: {0}")]
    NotFound(String),

    #[error("invalid arguments: {0}")]
    InvalidArgs(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("denied: {0}")]
    Denied(String),
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;
    async fn execute(&self, args: serde_json::Value) -> Result<ToolOutput, ToolError>;
}
