//! `recall` tool — search and retrieve memories from the cross-session store.
//!
//! Searches memories by substring match on key or value, or retrieves by exact
//! key. Returns matching memories ranked by access count and recency.

use grokrs_cap::WorkspaceRoot;
use grokrs_policy::Effect;
use serde_json::json;

use crate::error::ToolError;
use crate::{Classify, ToolSpec};

/// Input for the `recall` tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RecallInput {
    /// Search query: substring match on key or value.
    /// If empty or omitted, lists all memories.
    #[serde(default)]
    pub query: String,
    /// Optional: filter by category ("fact", "decision", "preference").
    #[serde(default)]
    pub category: Option<String>,
    /// Maximum number of results to return (default: 10).
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    10
}

impl Classify for RecallInput {
    fn classify(&self) -> Result<Vec<Effect>, ToolError> {
        // Reading from the local store is classified as FsRead on the store path.
        let wp = grokrs_cap::WorkspacePath::new(".grokrs/state.db")?;
        Ok(vec![Effect::FsRead(wp)])
    }
}

/// Searches and retrieves memories from the cross-session store.
#[derive(Debug, Clone)]
pub struct RecallTool;

impl ToolSpec for RecallTool {
    type Input = RecallInput;
    type Output = String;

    fn name(&self) -> &'static str {
        "recall"
    }

    fn description(&self) -> &str {
        "Search and retrieve memories from the cross-session store. \
         Finds memories by substring match on key or value. \
         Results are ranked by access frequency and recency."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query: substring match on memory key or value. Empty returns all memories."
                },
                "category": {
                    "type": "string",
                    "enum": ["fact", "decision", "preference"],
                    "description": "Optional category filter"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results (default: 10)",
                    "default": 10
                }
            },
            "required": [],
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

        let store = Store::open(root.as_path())
            .map_err(|e| ToolError::Other(format!("failed to open store: {e}")))?;

        let mem = store.memories();
        let records = if input.query.is_empty() {
            // List all, optionally filtered by category.
            let cat_filter = if let Some(ref cat_str) = input.category {
                Some(MemoryCategory::parse(cat_str).map_err(|e| {
                    ToolError::Other(format!("invalid category '{}': {e}", cat_str))
                })?)
            } else {
                None
            };
            mem.list(cat_filter)
                .map_err(|e| ToolError::Other(format!("failed to list memories: {e}")))?
        } else {
            mem.search(&input.query)
                .map_err(|e| ToolError::Other(format!("failed to search memories: {e}")))?
        };

        // Apply limit.
        let limited: Vec<_> = records.into_iter().take(input.limit).collect();

        if limited.is_empty() {
            return Ok("No memories found.".to_string());
        }

        // Format as readable output.
        let mut output = format!(
            "Found {} memor{}:\n",
            limited.len(),
            if limited.len() == 1 { "y" } else { "ies" }
        );
        for record in &limited {
            output.push_str(&format!(
                "\n- [{}] {}: {} (accessed {} time{})",
                record.category,
                record.key,
                record.value,
                record.access_count,
                if record.access_count == 1 { "" } else { "s" },
            ));
        }

        Ok(output)
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
    fn classify_produces_fs_read() {
        let input = RecallInput {
            query: "test".into(),
            category: None,
            limit: 10,
        };
        let effects = input.classify().unwrap();
        assert_eq!(effects.len(), 1);
        assert!(matches!(&effects[0], Effect::FsRead(_)));
    }

    #[tokio::test]
    async fn recall_empty_store() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        let result = RecallTool
            .execute(
                RecallInput {
                    query: "anything".into(),
                    category: None,
                    limit: 10,
                },
                &root,
            )
            .await
            .unwrap();

        assert_eq!(result, "No memories found.");
    }

    #[tokio::test]
    async fn recall_finds_saved_memories() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        // Save some memories first.
        {
            let store = grokrs_store::Store::open(dir.path()).unwrap();
            let mem = store.memories();
            mem.save(
                "rust-edition",
                "2021",
                grokrs_store::memory::MemoryCategory::Fact,
            )
            .unwrap();
            mem.save(
                "formatter",
                "rustfmt",
                grokrs_store::memory::MemoryCategory::Preference,
            )
            .unwrap();
        }

        let result = RecallTool
            .execute(
                RecallInput {
                    query: "rust".into(),
                    category: None,
                    limit: 10,
                },
                &root,
            )
            .await
            .unwrap();

        assert!(result.contains("rust-edition"));
        assert!(result.contains("rustfmt")); // "rust" matches "rustfmt" in value
    }

    #[tokio::test]
    async fn recall_respects_limit() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        {
            let store = grokrs_store::Store::open(dir.path()).unwrap();
            let mem = store.memories();
            for i in 0..5 {
                mem.save(
                    &format!("key-{i}"),
                    &format!("value-{i}"),
                    grokrs_store::memory::MemoryCategory::Fact,
                )
                .unwrap();
            }
        }

        let result = RecallTool
            .execute(
                RecallInput {
                    query: "key".into(),
                    category: None,
                    limit: 2,
                },
                &root,
            )
            .await
            .unwrap();

        assert!(result.contains("Found 2 memories"));
    }

    #[tokio::test]
    async fn recall_list_all_with_empty_query() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        {
            let store = grokrs_store::Store::open(dir.path()).unwrap();
            let mem = store.memories();
            mem.save("a", "va", grokrs_store::memory::MemoryCategory::Fact)
                .unwrap();
            mem.save("b", "vb", grokrs_store::memory::MemoryCategory::Decision)
                .unwrap();
        }

        let result = RecallTool
            .execute(
                RecallInput {
                    query: String::new(),
                    category: None,
                    limit: 10,
                },
                &root,
            )
            .await
            .unwrap();

        assert!(result.contains("Found 2 memories"));
    }

    #[test]
    fn has_description_and_schema() {
        let tool = RecallTool;
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["query"].is_object());
    }
}
