use async_trait::async_trait;
use serde_json::json;

use crate::traits::{Tool, ToolError, ToolOutput, ToolRisk};

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

    fn describe(&self, args: &serde_json::Value) -> String {
        args.get("path").and_then(|v| v.as_str()).unwrap_or("<invalid args>").to_string()
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

    fn risk(&self, _args: &serde_json::Value) -> ToolRisk {
        ToolRisk::Mutating
    }

    fn describe(&self, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("<invalid args>");
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
        format!("write {} bytes to {path}:\n```\n{content}\n```", content.len())
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

    fn describe(&self, args: &serde_json::Value) -> String {
        args.get("path").and_then(|v| v.as_str()).unwrap_or("<invalid args>").to_string()
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
#[path = "fs_test.rs"]
mod tests;
