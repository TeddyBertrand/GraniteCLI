use serde_json::json;

use crate::fs::{ListDirTool, ReadFileTool, WriteFileTool};
use crate::traits::{Tool, ToolError};

#[tokio::test]
async fn reads_file_contents() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("hello.txt");
    tokio::fs::write(&file_path, "hello world").await.unwrap();

    let tool = ReadFileTool;
    let out = tool
        .execute(json!({ "path": file_path.to_str().unwrap() }))
        .await
        .unwrap();

    assert_eq!(out.content, "hello world");
}

#[tokio::test]
async fn missing_path_arg_is_invalid_args() {
    let tool = ReadFileTool;
    let err = tool.execute(json!({})).await.unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(_)));
}

#[tokio::test]
async fn nonexistent_file_is_io_error() {
    let tool = ReadFileTool;
    let err = tool
        .execute(json!({ "path": "/nonexistent/path/does-not-exist.txt" }))
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Io(_)));
}

#[tokio::test]
async fn writes_file_contents() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("out.txt");

    let tool = WriteFileTool;
    tool.execute(json!({
        "path": file_path.to_str().unwrap(),
        "content": "hello write"
    }))
    .await
    .unwrap();

    let written = tokio::fs::read_to_string(&file_path).await.unwrap();
    assert_eq!(written, "hello write");
}

#[tokio::test]
async fn write_missing_args_is_invalid_args() {
    let tool = WriteFileTool;
    let err = tool
        .execute(json!({ "path": "/tmp/whatever.txt" }))
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(_)));
}

#[tokio::test]
async fn write_bad_dir_is_io_error() {
    let tool = WriteFileTool;
    let err = tool
        .execute(json!({
            "path": "/nonexistent/dir/out.txt",
            "content": "x"
        }))
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Io(_)));
}

#[tokio::test]
async fn lists_dir_entries_sorted() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("b.txt"), "").await.unwrap();
    tokio::fs::write(dir.path().join("a.txt"), "").await.unwrap();
    tokio::fs::create_dir(dir.path().join("sub")).await.unwrap();

    let tool = ListDirTool;
    let out = tool
        .execute(json!({ "path": dir.path().to_str().unwrap() }))
        .await
        .unwrap();

    assert_eq!(out.content, "a.txt\nb.txt\nsub");
}

#[tokio::test]
async fn list_dir_respects_gitignore() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join(".gitignore"), "ignored.txt\n")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("ignored.txt"), "")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("kept.txt"), "").await.unwrap();

    let tool = ListDirTool;
    let out = tool
        .execute(json!({ "path": dir.path().to_str().unwrap() }))
        .await
        .unwrap();

    assert_eq!(out.content, "kept.txt");
}

#[tokio::test]
async fn list_dir_missing_path_arg_is_invalid_args() {
    let tool = ListDirTool;
    let err = tool.execute(json!({})).await.unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(_)));
}

#[tokio::test]
async fn list_dir_nonexistent_path_is_io_error() {
    let tool = ListDirTool;
    let err = tool
        .execute(json!({ "path": "/nonexistent/dir/does-not-exist" }))
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Io(_)));
}

#[tokio::test]
async fn write_overwrites_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("out.txt");
    tokio::fs::write(&file_path, "old content").await.unwrap();

    let tool = WriteFileTool;
    tool.execute(json!({
        "path": file_path.to_str().unwrap(),
        "content": "new content"
    }))
    .await
    .unwrap();

    let written = tokio::fs::read_to_string(&file_path).await.unwrap();
    assert_eq!(written, "new content");
}

#[tokio::test]
async fn writes_empty_content() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("empty.txt");

    let tool = WriteFileTool;
    let out = tool
        .execute(json!({
            "path": file_path.to_str().unwrap(),
            "content": ""
        }))
        .await
        .unwrap();

    assert!(out.content.contains("wrote 0 bytes"));
    let written = tokio::fs::read_to_string(&file_path).await.unwrap();
    assert_eq!(written, "");
}

#[tokio::test]
async fn list_dir_on_empty_dir_returns_empty_content() {
    let dir = tempfile::tempdir().unwrap();
    let tool = ListDirTool;
    let out = tool
        .execute(json!({ "path": dir.path().to_str().unwrap() }))
        .await
        .unwrap();
    assert_eq!(out.content, "");
}
