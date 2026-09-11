use std::time::Duration;

use serde_json::json;
use tempfile::tempdir;

use crate::bash::BashTool;
use crate::traits::{Tool, ToolError, ToolRisk};

#[tokio::test]
async fn runs_command_and_captures_stdout() {
    let tool = BashTool::new();
    let out = tool
        .execute(json!({ "command": "echo hello" }))
        .await
        .unwrap();

    assert!(out.content.contains("exit code: 0"));
    assert!(out.content.contains("hello"));
}

#[tokio::test]
async fn captures_stderr_and_nonzero_exit_code() {
    let tool = BashTool::new();
    let out = tool
        .execute(json!({ "command": "echo oops 1>&2; exit 3" }))
        .await
        .unwrap();

    assert!(out.content.contains("exit code: 3"));
    assert!(out.content.contains("oops"));
}

#[tokio::test]
async fn missing_command_arg_is_invalid_args() {
    let tool = BashTool::new();
    let err = tool.execute(json!({})).await.unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(_)));
}

#[tokio::test]
async fn deny_listed_command_is_denied() {
    let tool = BashTool::new();
    let err = tool
        .execute(json!({ "command": "rm -rf /" }))
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Denied(_)));
}

#[tokio::test]
async fn risk_is_mutating_for_plain_command() {
    let tool = BashTool::new();
    assert_eq!(tool.risk(&json!({ "command": "echo hi" })), ToolRisk::Mutating);
}

#[tokio::test]
async fn risk_is_dangerous_for_destructive_command() {
    let tool = BashTool::new();
    assert_eq!(
        tool.risk(&json!({ "command": "sudo rm -r /tmp/whatever" })),
        ToolRisk::Dangerous
    );
}

#[tokio::test]
async fn deny_listed_git_force_push_is_denied() {
    let tool = BashTool::new();
    let err = tool
        .execute(json!({ "command": "git push --force origin main" }))
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Denied(_)));
}

#[tokio::test]
async fn custom_deny_pattern_is_enforced() {
    let tool = BashTool::new()
        .with_deny_patterns(&[r"\bcargo\s+publish\b"])
        .unwrap();
    let err = tool
        .execute(json!({ "command": "cargo publish" }))
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Denied(_)));
}

#[tokio::test]
async fn allow_pattern_permits_matching_command() {
    let tool = BashTool::new().with_allow_patterns(&[r"^echo\s"]).unwrap();
    let out = tool.execute(json!({ "command": "echo hi" })).await.unwrap();
    assert!(out.content.contains("hi"));
}

#[tokio::test]
async fn allow_pattern_denies_non_matching_command() {
    let tool = BashTool::new().with_allow_patterns(&[r"^echo\s"]).unwrap();
    let err = tool
        .execute(json!({ "command": "ls -la" }))
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Denied(_)));
}

#[tokio::test]
async fn deny_pattern_wins_even_if_allow_pattern_matches() {
    let tool = BashTool::new()
        .with_allow_patterns(&[r"^git\s"])
        .unwrap();
    let err = tool
        .execute(json!({ "command": "git reset --hard HEAD~1" }))
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Denied(_)));
}

#[tokio::test]
async fn invalid_pattern_regex_is_rejected_at_construction() {
    let err = BashTool::new().with_deny_patterns(&["(unclosed"]);
    assert!(err.is_err());
}

#[tokio::test]
async fn destructive_command_runs_directly_gating_is_agent_side() {
    let tool = BashTool::new();
    let out = tool
        .execute(json!({ "command": "kill -9 999999" }))
        .await
        .unwrap();
    assert!(out.content.contains("exit code:"));
}

#[tokio::test]
async fn non_destructive_command_runs() {
    let tool = BashTool::new();
    let out = tool.execute(json!({ "command": "echo hi" })).await.unwrap();
    assert!(out.content.contains("hi"));
}

#[tokio::test]
async fn sandboxed_command_runs_in_sandbox_dir() {
    let dir = tempdir().unwrap();
    let tool = BashTool::new().with_sandbox_dir(dir.path()).unwrap();
    let out = tool.execute(json!({ "command": "pwd" })).await.unwrap();
    let canonical = dir.path().canonicalize().unwrap();
    assert!(out.content.contains(canonical.to_str().unwrap()));
}

#[tokio::test]
async fn sandboxed_command_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let tool = BashTool::new().with_sandbox_dir(dir.path()).unwrap();
    let err = tool
        .execute(json!({ "command": "cat ../secret" }))
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Denied(_)));
}

#[tokio::test]
async fn command_exceeding_timeout_is_timeout_error() {
    let tool = BashTool::new().with_timeout(Duration::from_millis(50));
    let err = tool
        .execute(json!({ "command": "sleep 5" }))
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Timeout(_)));
}
