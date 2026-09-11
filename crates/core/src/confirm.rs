use async_trait::async_trait;
use tools::ToolRisk;

#[derive(Debug, Clone)]
pub struct ConfirmRequest {
    pub tool_name: String,
    pub risk: ToolRisk,
    pub detail: String,
}

#[async_trait]
pub trait ConfirmHook: Send + Sync {
    async fn confirm(&self, request: ConfirmRequest) -> bool;
}
