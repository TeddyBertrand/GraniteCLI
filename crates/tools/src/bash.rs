use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde_json::json;
use tokio::process::Command;

use crate::traits::{Tool, ToolError, ToolOutput};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

const DEFAULT_DENY_PATTERNS: &[&str] = &[
    r"rm\s+(-\w*r\w*f\w*|-\w*f\w*r\w*)\s+/(\s|$|\*)",
    r"rm\s+(-\w*r\w*f\w*|-\w*f\w*r\w*)\s+~",
    r":\(\)\s*\{\s*:\s*\|\s*:\s*&?\s*\}\s*;\s*:",
    r"mkfs(\.\w+)?\s",
    r"dd\s+if=",
    r">\s*/dev/sd\w*",
    r"\bshutdown\b",
    r"\breboot\b",
    r"\bpoweroff\b",
    r"chmod\s+-R\s+777\s+/",
    r"chown\s+-R\s+.*\s+/(\s|$)",
    r"(curl|wget)\s+.*\|\s*(sh|bash)\b",
    r"/dev/tcp/",
    r"git\s+push\s+.*--force",
    r"git\s+reset\s+--hard",
    r"git\s+clean\s+-\w*f\w*d\w*",
    r"\bDROP\s+TABLE\b",
    r"\bTRUNCATE\s+TABLE\b",
];

const DESTRUCTIVE_PATTERNS: &[&str] = &[
    r"rm\s+-\w*r\w*",
    r"\bsudo\b",
    r"chmod\s+-R\b",
    r"chown\s+-R\b",
    r">\s*[^&]",
    r"\bkill\s+-9\b",
    r"(npm|cargo)\s+publish\b",
    r"docker\s+(rm|rmi)\s+-f\b",
    r"kubectl\s+delete\b",
    r"git\s+push\s+.*--force",
    r"(apt|apt-get)\s+(remove|purge)\b",
    r"pip\d?\s+uninstall\b",
];

fn compile(patterns: &[&str]) -> Vec<Regex> {
    patterns
        .iter()
        .map(|p| Regex::new(&format!("(?i){p}")).expect("built-in pattern must compile"))
        .collect()
}

pub type ConfirmHook = Arc<dyn Fn(&str) -> bool + Send + Sync>;

pub struct BashTool {
    timeout: Duration,
    confirm: Option<ConfirmHook>,
    deny_patterns: Vec<Regex>,
    allow_patterns: Vec<Regex>,
    destructive_patterns: Vec<Regex>,
    sandbox_dir: Option<std::path::PathBuf>,
}

impl BashTool {
    pub fn new() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            confirm: None,
            deny_patterns: compile(DEFAULT_DENY_PATTERNS),
            allow_patterns: Vec::new(),
            destructive_patterns: compile(DESTRUCTIVE_PATTERNS),
            sandbox_dir: None,
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

    pub fn with_deny_patterns(mut self, patterns: &[&str]) -> Result<Self, regex::Error> {
        for p in patterns {
            self.deny_patterns.push(Regex::new(&format!("(?i){p}"))?);
        }
        Ok(self)
    }

    pub fn with_allow_patterns(mut self, patterns: &[&str]) -> Result<Self, regex::Error> {
        for p in patterns {
            self.allow_patterns.push(Regex::new(&format!("(?i){p}"))?);
        }
        Ok(self)
    }

    pub fn with_sandbox_dir(mut self, dir: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        self.sandbox_dir = Some(dir.as_ref().canonicalize()?);
        Ok(self)
    }

    fn is_destructive(&self, cmd: &str) -> bool {
        self.destructive_patterns.iter().any(|p| p.is_match(cmd))
    }

    fn check_sandbox_escape(cmd: &str) -> Result<(), ToolError> {
        if cmd.contains("..") {
            return Err(ToolError::Denied(
                "command contains '..' path traversal, rejected under sandbox".to_string(),
            ));
        }
        Ok(())
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

        if self.sandbox_dir.is_some() {
            Self::check_sandbox_escape(command)?;
        }

        if self.is_destructive(command) && self.confirm.is_none() {
            return Err(ToolError::Denied(
                "destructive command requires a confirmation hook".to_string(),
            ));
        }

        if let Some(confirm) = &self.confirm {
            if !confirm(command) {
                return Err(ToolError::Denied("rejected by confirmation hook".to_string()));
            }
        }

        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(dir) = &self.sandbox_dir {
            cmd.current_dir(dir);
        }

        let child = cmd.spawn()?;

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
