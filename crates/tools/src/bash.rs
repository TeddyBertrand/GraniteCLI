use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde_json::json;
use tokio::process::Command;

use crate::traits::{Tool, ToolError, ToolOutput};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Built-in deny patterns, grouped by category. All case-insensitive.
/// Callers can layer more via `with_deny_patterns`; these always apply.
const DEFAULT_DENY_PATTERNS: &[&str] = &[
    // filesystem destruction
    r"rm\s+(-\w*r\w*f\w*|-\w*f\w*r\w*)\s+/(\s|$|\*)",
    r"rm\s+(-\w*r\w*f\w*|-\w*f\w*r\w*)\s+~",
    r":\(\)\s*\{\s*:\s*\|\s*:\s*&?\s*\}\s*;\s*:",
    r"mkfs(\.\w+)?\s",
    r"dd\s+if=",
    r">\s*/dev/sd\w*",
    // system/privilege
    r"\bshutdown\b",
    r"\breboot\b",
    r"\bpoweroff\b",
    r"chmod\s+-R\s+777\s+/",
    r"chown\s+-R\s+.*\s+/(\s|$)",
    // network exfil / remote exec
    r"(curl|wget)\s+.*\|\s*(sh|bash)\b",
    r"/dev/tcp/",
    // git destructive
    r"git\s+push\s+.*--force",
    r"git\s+reset\s+--hard",
    r"git\s+clean\s+-\w*f\w*d\w*",
    // db destructive
    r"\bDROP\s+TABLE\b",
    r"\bTRUNCATE\s+TABLE\b",
];

fn compile(patterns: &[&str]) -> Vec<Regex> {
    patterns
        .iter()
        .map(|p| Regex::new(&format!("(?i){p}")).expect("built-in pattern must compile"))
        .collect()
}

/// Called with the raw command string before exec; return `false` to deny.
/// Lets the CLI wire in an interactive "allow this command?" prompt.
pub type ConfirmHook = Arc<dyn Fn(&str) -> bool + Send + Sync>;

pub struct BashTool {
    timeout: Duration,
    confirm: Option<ConfirmHook>,
    deny_patterns: Vec<Regex>,
    allow_patterns: Vec<Regex>,
}

impl BashTool {
    pub fn new() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            confirm: None,
            deny_patterns: compile(DEFAULT_DENY_PATTERNS),
            allow_patterns: Vec::new(),
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

    /// Adds extra deny patterns (regexes, case-insensitive) on top of the
    /// built-in defaults, which always stay active.
    pub fn with_deny_patterns(mut self, patterns: &[&str]) -> Result<Self, regex::Error> {
        for p in patterns {
            self.deny_patterns.push(Regex::new(&format!("(?i){p}"))?);
        }
        Ok(self)
    }

    /// Restricts execution to commands matching at least one allow pattern.
    /// When empty (the default), every non-denied command is allowed.
    /// Deny patterns are still enforced even when a command matches an allow
    /// pattern.
    pub fn with_allow_patterns(mut self, patterns: &[&str]) -> Result<Self, regex::Error> {
        for p in patterns {
            self.allow_patterns.push(Regex::new(&format!("(?i){p}"))?);
        }
        Ok(self)
    }

    fn check_patterns(&self, cmd: &str) -> Result<(), ToolError> {
        for pattern in &self.deny_patterns {
            if pattern.is_match(cmd) {
                return Err(ToolError::Denied(format!(
                    "command matches deny-list pattern: {pattern}"
                )));
            }
        }

        if !self.allow_patterns.is_empty() && !self.allow_patterns.iter().any(|p| p.is_match(cmd))
        {
            return Err(ToolError::Denied(
                "command does not match any allow-list pattern".to_string(),
            ));
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

        self.check_patterns(command)?;

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
