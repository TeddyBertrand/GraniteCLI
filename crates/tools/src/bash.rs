use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tokio::process::Command;

use crate::traits::{Tool, ToolError, ToolOutput};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Substrings that are always rejected before exec, regardless of confirm hook.
const DENY_SUBSTRINGS: &[&str] = &[
    "rm -rf /",
    "rm -rf /*",
    ":(){ :|:& };:",
    "mkfs",
    "dd if=",
    "> /dev/sda",
    "shutdown",
    "reboot",
    "poweroff",
    ":(){:|:&};:",
];

/// Called with the raw command string before exec; return `false` to deny.
/// Lets the CLI wire in an interactive "allow this command?" prompt.
pub type ConfirmHook = Arc<dyn Fn(&str) -> bool + Send + Sync>;

pub struct BashTool {
    timeout: Duration,
    confirm: Option<ConfirmHook>,
}

impl BashTool {
    pub fn new() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            confirm: None,
        }
    }

    pub fn with_confirm_hook(mut self, hook: ConfirmHook) -> Self {
        self.confirm = Some(hook);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn check_denied(cmd: &str) -> Result<(), ToolError> {
        for pattern in DENY_SUBSTRINGS {
            if cmd.contains(pattern) {
                return Err(ToolError::Denied(format!(
                    "command matches deny-list pattern: {pattern}"
                )));
            }
        }
        Ok(())
    }
}

impl Default for BashTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its stdout/stderr/exit code."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'command' field".to_string()))?;

        Self::check_denied(command)?;

        if let Some(confirm) = &self.confirm {
            if !confirm(command) {
                return Err(ToolError::Denied("rejected by confirmation hook".to_string()));
            }
        }

        let child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let output = match tokio::time::timeout(self.timeout, child.wait_with_output()).await {
            Ok(result) => result?,
            Err(_) => return Err(ToolError::Timeout(self.timeout)),
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let exit_code = output.status.code().unwrap_or(-1);

        Ok(ToolOutput {
            content: format!("exit code: {exit_code}\nstdout:\n{stdout}\nstderr:\n{stderr}"),
        })
    }
}

#[cfg(test)]
#[path = "bash_test.rs"]
mod tests;
