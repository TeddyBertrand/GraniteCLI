use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::bash::BashTool;
use crate::traits::{Tool, ToolError};

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
async fn confirm_hook_can_deny() {
    let tool = BashTool::new().with_confirm_hook(Arc::new(|_cmd: &str| false));
    let err = tool
        .execute(json!({ "command": "echo hi" }))
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Denied(_)));
}

#[tokio::test]
async fn confirm_hook_is_called_with_command_and_can_allow() {
    let called = Arc::new(AtomicBool::new(false));
    let called_clone = called.clone();
    let tool = BashTool::new().with_confirm_hook(Arc::new(move |cmd: &str| {
        called_clone.store(true, Ordering::SeqCst);
        cmd == "echo hi"
    }));

    let out = tool.execute(json!({ "command": "echo hi" })).await.unwrap();

    assert!(called.load(Ordering::SeqCst));
    assert!(out.content.contains("hi"));
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
