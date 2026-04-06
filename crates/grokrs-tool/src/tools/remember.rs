//! `remember` tool — persist a memory to the cross-session store.
//!
//! Saves a key-value pair with a category (fact, decision, preference) to the
//! workspace-local SQLite store. If a memory with the same key already exists,
//! it is updated.

use grokrs_cap::WorkspaceRoot;
use grokrs_policy::Effect;
use serde_json::json;

use crate::error::ToolError;
use crate::{Classify, ToolSpec};

/// Input for the `remember` tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RememberInput {
    /// Unique key for this memory.
    pub key: String,
    /// The memory content to store.
    pub value: String,
    /// Category: "fact", "decision", or "preference". Defaults to "fact".
    #[serde(default = "default_category")]
    pub category: String,
}

fn default_category() -> String {
    "fact".to_owned()
}

impl Classify for RememberInput {
    fn classify(&self) -> Result<Vec<Effect>, ToolError> {
        // Writing to the local store is classified as FsWrite on the store path.
        let wp = grokrs_cap::WorkspacePath::new(".grokrs/state.db")?;
        Ok(vec![Effect::FsWrite(wp)])
    }
}

/// Saves a key-value memory to the cross-session store.
#[derive(Debug, Clone)]
pub struct RememberTool;

impl ToolSpec for RememberTool {
    type Input = RememberInput;
    type Output = String;

    fn name(&self) -> &'static str {
        "remember"
    }

    fn description(&self) -> &str {
        "Save a memory (key-value pair) to the cross-session store. \
         Use this to persist facts, decisions, or preferences that should \
         be recalled in future sessions. Categories: fact, decision, preference."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Unique key identifying this memory (e.g., 'rust-edition', 'preferred-formatter')"
                },
                "value": {
                    "type": "string",
                    "description": "The memory content to store"
                },
                "category": {
                    "type": "string",
                    "enum": ["fact", "decision", "preference"],
                    "description": "Category of memory: fact (codebase observation), decision (remembered choice), preference (user preference). Defaults to 'fact'."
                }
            },
            "required": ["key", "value"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        input: Self::Input,
        root: &WorkspaceRoot,
    ) -> Result<Self::Output, ToolError> {
        use grokrs_store::Store;
        use grokrs_store::memory::MemoryCategory;

        let category = MemoryCategory::parse(&input.category)
            .map_err(|e| ToolError::Other(format!("invalid category '{}': {e}", input.category)))?;

        let store = Store::open(root.as_path())
            .map_err(|e| ToolError::Other(format!("failed to open store: {e}")))?;

        // Evict if we're at the memory limit before saving.
        // Default limit is 50; this is applied here as a safety net.
        // The configurable limit is handled at the agent level.
        let mem = store.memories();
        mem.evict(50)
            .map_err(|e| ToolError::Other(format!("failed to evict memories: {e}")))?;

        mem.save(&input.key, &input.value, category)
            .map_err(|e| ToolError::Other(format!("failed to save memory: {e}")))?;

        Ok(format!(
            "Saved memory: key='{}', category='{}', value='{}'",
            input.key,
            input.category,
            truncate(&input.value, 100),
        ))
    }
}

/// Truncate a string for display, appending "..." if it exceeds `max_len`.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut i = max_len;
        while i > 0 && !s.is_char_boundary(i) {
            i -= 1;
        }
        format!("{}...", &s[..i])
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
        let input = RememberInput {
            key: "test".into(),
            value: "val".into(),
            category: "fact".into(),
        };
        let effects = input.classify().unwrap();
        assert_eq!(effects.len(), 1);
        assert!(matches!(&effects[0], Effect::FsWrite(_)));
    }

    #[tokio::test]
    async fn remember_saves_memory() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        let result = RememberTool
            .execute(
                RememberInput {
                    key: "test-key".into(),
                    value: "test-value".into(),
                    category: "fact".into(),
                },
                &root,
            )
            .await
            .unwrap();

        assert!(result.contains("test-key"));

        // Verify it was persisted.
        let store = grokrs_store::Store::open(dir.path()).unwrap();
        let record = store.memories().get("test-key").unwrap().unwrap();
        assert_eq!(record.value, "test-value");
        assert_eq!(record.category, "fact");
    }

    #[tokio::test]
    async fn remember_rejects_invalid_category() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        let err = RememberTool
            .execute(
                RememberInput {
                    key: "k".into(),
                    value: "v".into(),
                    category: "invalid".into(),
                },
                &root,
            )
            .await
            .unwrap_err();

        assert!(matches!(err, ToolError::Other(_)));
    }

    #[test]
    fn default_category_is_fact() {
        let input: RememberInput = serde_json::from_str(r#"{"key": "k", "value": "v"}"#).unwrap();
        assert_eq!(input.category, "fact");
    }

    #[test]
    fn has_description_and_schema() {
        let tool = RememberTool;
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["key"].is_object());
        assert!(schema["properties"]["value"].is_object());
    }
}
