use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRisk {
    Safe,
    Mutating,
    Dangerous,
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

    #[error("timed out after {0:?}")]
    Timeout(std::time::Duration),
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;

    fn risk(&self, _args: &serde_json::Value) -> ToolRisk {
        ToolRisk::Safe
    }

    fn describe(&self, args: &serde_json::Value) -> String {
        args.to_string()
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolOutput, ToolError>;
}
