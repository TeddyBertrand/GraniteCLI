use async_trait::async_trait;
use serde_json::json;

use crate::traits::{Tool, ToolError, ToolOutput};

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file at the given path."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'path' field".to_string()))?;

        let content = tokio::fs::read_to_string(path).await?;

        Ok(ToolOutput { content })
    }
}

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file at the given path, creating or overwriting it."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'path' field".to_string()))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'content' field".to_string()))?;

        tokio::fs::write(path, content).await?;

        Ok(ToolOutput {
            content: format!("wrote {} bytes to {}", content.len(), path),
        })
    }
}

pub struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn description(&self) -> &str {
        "List entries of a directory, respecting .gitignore rules."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the directory to list"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'path' field".to_string()))?
            .to_string();

        let entries = tokio::task::spawn_blocking(move || -> Result<Vec<String>, std::io::Error> {
            let mut names = Vec::new();
            let walker = ignore::WalkBuilder::new(&path)
                .max_depth(Some(1))
                .require_git(false)
                .build();

            for entry in walker {
                let entry = entry.map_err(std::io::Error::other)?;
                if entry.depth() == 0 {
                    continue;
                }
                names.push(entry.file_name().to_string_lossy().into_owned());
            }

            names.sort();
            Ok(names)
        })
        .await
        .map_err(std::io::Error::other)??;

        Ok(ToolOutput {
            content: entries.join("\n"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
