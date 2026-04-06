//! `forget` tool — delete a memory from the cross-session store.
//!
//! Removes a memory by exact key match. Returns whether the memory was found
//! and deleted.

use grokrs_cap::WorkspaceRoot;
use grokrs_policy::Effect;
use serde_json::json;

use crate::error::ToolError;
use crate::{Classify, ToolSpec};

/// Input for the `forget` tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ForgetInput {
    /// The exact key of the memory to delete.
    pub key: String,
}

impl Classify for ForgetInput {
    fn classify(&self) -> Result<Vec<Effect>, ToolError> {
        // Deleting from the local store is classified as FsWrite on the store path.
        let wp = grokrs_cap::WorkspacePath::new(".grokrs/state.db")?;
        Ok(vec![Effect::FsWrite(wp)])
    }
}

/// Deletes a memory from the cross-session store by exact key.
#[derive(Debug, Clone)]
pub struct ForgetTool;

impl ToolSpec for ForgetTool {
    type Input = ForgetInput;
    type Output = String;

    fn name(&self) -> &'static str {
        "forget"
    }

    fn description(&self) -> &'static str {
        "Delete a memory from the cross-session store by its exact key. \
         Use this to remove outdated or incorrect memories."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "The exact key of the memory to delete"
                }
            },
            "required": ["key"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        input: Self::Input,
        root: &WorkspaceRoot,
    ) -> Result<Self::Output, ToolError> {
        use grokrs_store::Store;

        let store = Store::open(root.as_path())
            .map_err(|e| ToolError::Other(format!("failed to open store: {e}")))?;

        let deleted = store
            .memories()
            .delete(&input.key)
            .map_err(|e| ToolError::Other(format!("failed to delete memory: {e}")))?;

        if deleted {
            Ok(format!("Deleted memory with key '{}'.", input.key))
        } else {
            Ok(format!("No memory found with key '{}'.", input.key))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn workspace(dir: &TempDir) -> WorkspaceRoot {
        WorkspaceRoot::new(dir.path()).unwrap()
    }

    #[test]
    fn classify_produces_fs_write() {
        let input = ForgetInput { key: "test".into() };
        let effects = input.classify().unwrap();
        assert_eq!(effects.len(), 1);
        assert!(matches!(&effects[0], Effect::FsWrite(_)));
    }

    #[tokio::test]
    async fn forget_deletes_existing_memory() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        // Save a memory first.
        {
            let store = grokrs_store::Store::open(dir.path()).unwrap();
            store
                .memories()
                .save(
                    "test-key",
                    "test-value",
                    grokrs_store::memory::MemoryCategory::Fact,
                )
                .unwrap();
        }

        let result = ForgetTool
            .execute(
                ForgetInput {
                    key: "test-key".into(),
                },
                &root,
            )
            .await
            .unwrap();

        assert!(result.contains("Deleted"));

        // Verify it's gone.
        let store = grokrs_store::Store::open(dir.path()).unwrap();
        assert!(store.memories().get("test-key").unwrap().is_none());
    }

    #[tokio::test]
    async fn forget_nonexistent_returns_not_found() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        let result = ForgetTool
            .execute(
                ForgetInput {
                    key: "nonexistent".into(),
                },
                &root,
            )
            .await
            .unwrap();

        assert!(result.contains("No memory found"));
    }

    #[test]
    fn has_description_and_schema() {
        let tool = ForgetTool;
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["key"].is_object());
    }
}
