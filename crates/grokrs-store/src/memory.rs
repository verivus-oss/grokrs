//! Cross-session agent memory persistence: CRUD, search, and eviction.
//!
//! `MemoryRepo` is a borrowed handle into the `Store`'s connection. It provides
//! operations for saving, retrieving, searching, and evicting agent memories.
//! Memories are simple key-value pairs with a category tag, timestamps, and an
//! access count used for ranking (most-accessed + most-recent first).

use rusqlite::{Connection, params};

use crate::StoreError;
use crate::session::now;

/// Category of a memory entry.
///
/// Stored as a plain string in SQLite. Valid values are `"fact"`, `"decision"`,
/// and `"preference"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryCategory {
    /// A factual observation about the codebase, project, or environment.
    Fact,
    /// A decision that was made and should be remembered.
    Decision,
    /// A user preference or working style.
    Preference,
}

impl MemoryCategory {
    /// Return the string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Decision => "decision",
            Self::Preference => "preference",
        }
    }

    /// Parse from a string value (case-insensitive).
    ///
    /// This is a convenience wrapper around the `FromStr` implementation that
    /// returns `StoreError` instead of `MemoryCategoryParseError`.
    pub fn parse(s: &str) -> Result<Self, StoreError> {
        s.parse::<Self>()
            .map_err(|e| StoreError::Migration(e.to_string()))
    }
}

/// Error returned when parsing an invalid memory category string.
#[derive(Debug, Clone)]
pub struct MemoryCategoryParseError(String);

impl std::fmt::Display for MemoryCategoryParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid memory category: '{}' (expected fact, decision, or preference)",
            self.0
        )
    }
}

impl std::error::Error for MemoryCategoryParseError {}

impl std::str::FromStr for MemoryCategory {
    type Err = MemoryCategoryParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "fact" => Ok(Self::Fact),
            "decision" => Ok(Self::Decision),
            "preference" => Ok(Self::Preference),
            _ => Err(MemoryCategoryParseError(s.to_owned())),
        }
    }
}

impl std::fmt::Display for MemoryCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A memory record as stored in the `memories` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRecord {
    /// Auto-incremented memory identifier.
    pub id: i64,
    /// Unique key for this memory.
    pub key: String,
    /// The memory value / content.
    pub value: String,
    /// Category: fact, decision, or preference.
    pub category: String,
    /// RFC 3339 timestamp of creation.
    pub created_at: String,
    /// RFC 3339 timestamp of last update.
    pub updated_at: String,
    /// Number of times this memory has been accessed (get or search hit).
    pub access_count: i64,
}

/// Borrowed handle for memory operations on the store's connection.
pub struct MemoryRepo<'a> {
    conn: &'a Connection,
}

impl<'a> MemoryRepo<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Save a memory (insert or update if the key already exists).
    ///
    /// If a memory with the same key exists, its value, category, and
    /// `updated_at` timestamp are updated. Otherwise a new row is inserted.
    pub fn save(&self, key: &str, value: &str, category: MemoryCategory) -> Result<(), StoreError> {
        let now_ts = now();
        let cat = category.as_str();

        self.conn
            .execute(
                "INSERT INTO memories (key, value, category, created_at, updated_at, access_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0)
                 ON CONFLICT(key) DO UPDATE SET
                     value = excluded.value,
                     category = excluded.category,
                     updated_at = excluded.updated_at",
                params![key, value, cat, &now_ts, &now_ts],
            )
            .map_err(StoreError::Sql)?;

        Ok(())
    }

    /// Retrieve a memory by exact key. Increments `access_count` on hit.
    ///
    /// Returns `None` if no memory with the given key exists.
    pub fn get(&self, key: &str) -> Result<Option<MemoryRecord>, StoreError> {
        // First, try to read.
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, key, value, category, created_at, updated_at, access_count
                 FROM memories WHERE key = ?1",
            )
            .map_err(StoreError::Sql)?;

        let mut rows = stmt
            .query_map(params![key], |row| {
                Ok(MemoryRecord {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    value: row.get(2)?,
                    category: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    access_count: row.get(6)?,
                })
            })
            .map_err(StoreError::Sql)?;

        match rows.next() {
            Some(Ok(mut record)) => {
                // Increment access_count.
                self.conn
                    .execute(
                        "UPDATE memories SET access_count = access_count + 1 WHERE key = ?1",
                        params![key],
                    )
                    .map_err(StoreError::Sql)?;
                record.access_count += 1;
                Ok(Some(record))
            }
            Some(Err(e)) => Err(StoreError::Sql(e)),
            None => Ok(None),
        }
    }

    /// List all memories, optionally filtered by category.
    ///
    /// Results are ordered by access_count DESC, updated_at DESC (most relevant first).
    pub fn list(&self, category: Option<MemoryCategory>) -> Result<Vec<MemoryRecord>, StoreError> {
        let records = if let Some(cat) = category {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT id, key, value, category, created_at, updated_at, access_count
                     FROM memories
                     WHERE category = ?1
                     ORDER BY access_count DESC, updated_at DESC",
                )
                .map_err(StoreError::Sql)?;

            let rows = stmt
                .query_map(params![cat.as_str()], |row| {
                    Ok(MemoryRecord {
                        id: row.get(0)?,
                        key: row.get(1)?,
                        value: row.get(2)?,
                        category: row.get(3)?,
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                        access_count: row.get(6)?,
                    })
                })
                .map_err(StoreError::Sql)?;

            let mut results = Vec::new();
            for row in rows {
                results.push(row.map_err(StoreError::Sql)?);
            }
            results
        } else {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT id, key, value, category, created_at, updated_at, access_count
                     FROM memories
                     ORDER BY access_count DESC, updated_at DESC",
                )
                .map_err(StoreError::Sql)?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(MemoryRecord {
                        id: row.get(0)?,
                        key: row.get(1)?,
                        value: row.get(2)?,
                        category: row.get(3)?,
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                        access_count: row.get(6)?,
                    })
                })
                .map_err(StoreError::Sql)?;

            let mut results = Vec::new();
            for row in rows {
                results.push(row.map_err(StoreError::Sql)?);
            }
            results
        };

        Ok(records)
    }

    /// Search memories by substring match on key or value.
    ///
    /// Matching records have their `access_count` incremented. Results are
    /// ordered by access_count DESC, updated_at DESC.
    pub fn search(&self, query: &str) -> Result<Vec<MemoryRecord>, StoreError> {
        let pattern = format!("%{query}%");
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, key, value, category, created_at, updated_at, access_count
                 FROM memories
                 WHERE key LIKE ?1 OR value LIKE ?1
                 ORDER BY access_count DESC, updated_at DESC",
            )
            .map_err(StoreError::Sql)?;

        let rows = stmt
            .query_map(params![&pattern], |row| {
                Ok(MemoryRecord {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    value: row.get(2)?,
                    category: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    access_count: row.get(6)?,
                })
            })
            .map_err(StoreError::Sql)?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(StoreError::Sql)?);
        }

        // Increment access_count for all matched records.
        if !results.is_empty() {
            self.conn
                .execute(
                    "UPDATE memories SET access_count = access_count + 1
                     WHERE key LIKE ?1 OR value LIKE ?1",
                    params![&pattern],
                )
                .map_err(StoreError::Sql)?;

            // Reflect the increment in the returned records.
            for record in &mut results {
                record.access_count += 1;
            }
        }

        Ok(results)
    }

    /// Delete a memory by exact key.
    ///
    /// Returns `true` if a memory was deleted, `false` if no memory with the
    /// given key existed.
    pub fn delete(&self, key: &str) -> Result<bool, StoreError> {
        let affected = self
            .conn
            .execute("DELETE FROM memories WHERE key = ?1", params![key])
            .map_err(StoreError::Sql)?;
        Ok(affected > 0)
    }

    /// Return the total number of memories.
    pub fn count(&self) -> Result<i64, StoreError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
            .map_err(StoreError::Sql)?;
        Ok(count)
    }

    /// Evict the oldest, least-accessed memories to bring the total count
    /// at or below `max_entries`.
    ///
    /// Eviction order: lowest access_count first, then oldest updated_at.
    /// Returns the number of evicted memories.
    pub fn evict(&self, max_entries: i64) -> Result<i64, StoreError> {
        let current = self.count()?;
        if current <= max_entries {
            return Ok(0);
        }

        let to_evict = current - max_entries;

        // Delete the least-valuable memories (lowest access_count, oldest updated_at).
        self.conn
            .execute(
                "DELETE FROM memories WHERE id IN (
                    SELECT id FROM memories
                    ORDER BY access_count ASC, updated_at ASC
                    LIMIT ?1
                )",
                params![to_evict],
            )
            .map_err(StoreError::Sql)?;

        Ok(to_evict)
    }

    /// Retrieve the top-N memories ranked by access_count DESC, updated_at DESC.
    ///
    /// Used for building the agent system prompt with the most relevant memories.
    pub fn top_n(&self, n: i64) -> Result<Vec<MemoryRecord>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, key, value, category, created_at, updated_at, access_count
                 FROM memories
                 ORDER BY access_count DESC, updated_at DESC
                 LIMIT ?1",
            )
            .map_err(StoreError::Sql)?;

        let rows = stmt
            .query_map(params![n], |row| {
                Ok(MemoryRecord {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    value: row.get(2)?,
                    category: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    access_count: row.get(6)?,
                })
            })
            .map_err(StoreError::Sql)?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(StoreError::Sql)?);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    fn open_test_store() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        (tmp, store)
    }

    #[test]
    fn save_and_get_memory() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        mem.save("test-key", "test-value", MemoryCategory::Fact)
            .unwrap();

        let record = mem.get("test-key").unwrap().expect("memory should exist");
        assert_eq!(record.key, "test-key");
        assert_eq!(record.value, "test-value");
        assert_eq!(record.category, "fact");
        assert_eq!(record.access_count, 1); // get increments access_count
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        assert!(mem.get("nonexistent").unwrap().is_none());
    }

    #[test]
    fn save_upserts_existing_key() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        mem.save("key1", "value1", MemoryCategory::Fact).unwrap();
        mem.save("key1", "updated-value", MemoryCategory::Decision)
            .unwrap();

        let record = mem.get("key1").unwrap().expect("memory should exist");
        assert_eq!(record.value, "updated-value");
        assert_eq!(record.category, "decision");
    }

    #[test]
    fn list_all_memories() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        mem.save("a", "value-a", MemoryCategory::Fact).unwrap();
        mem.save("b", "value-b", MemoryCategory::Decision).unwrap();
        mem.save("c", "value-c", MemoryCategory::Preference)
            .unwrap();

        let all = mem.list(None).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn list_by_category() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        mem.save("a", "va", MemoryCategory::Fact).unwrap();
        mem.save("b", "vb", MemoryCategory::Decision).unwrap();
        mem.save("c", "vc", MemoryCategory::Fact).unwrap();

        let facts = mem.list(Some(MemoryCategory::Fact)).unwrap();
        assert_eq!(facts.len(), 2);
        for f in &facts {
            assert_eq!(f.category, "fact");
        }

        let decisions = mem.list(Some(MemoryCategory::Decision)).unwrap();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].key, "b");
    }

    #[test]
    fn search_by_key_substring() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        mem.save("rust-version", "1.80", MemoryCategory::Fact)
            .unwrap();
        mem.save("python-version", "3.12", MemoryCategory::Fact)
            .unwrap();
        mem.save("rust-edition", "2021", MemoryCategory::Fact)
            .unwrap();

        let results = mem.search("rust").unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.key.contains("rust")));
    }

    #[test]
    fn search_by_value_substring() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        mem.save("framework", "uses axum for HTTP", MemoryCategory::Fact)
            .unwrap();
        mem.save("editor", "prefers vim", MemoryCategory::Preference)
            .unwrap();

        let results = mem.search("axum").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "framework");
    }

    #[test]
    fn search_empty_returns_all_no_match() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        mem.save("a", "va", MemoryCategory::Fact).unwrap();

        let results = mem.search("zzz-nonexistent").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_increments_access_count() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        mem.save("key1", "value1", MemoryCategory::Fact).unwrap();

        let results = mem.search("key1").unwrap();
        assert_eq!(results[0].access_count, 1);

        // Search again — access_count should be 2.
        let results2 = mem.search("key1").unwrap();
        assert_eq!(results2[0].access_count, 2);
    }

    #[test]
    fn get_increments_access_count() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        mem.save("key1", "value1", MemoryCategory::Fact).unwrap();

        let r1 = mem.get("key1").unwrap().unwrap();
        assert_eq!(r1.access_count, 1);

        let r2 = mem.get("key1").unwrap().unwrap();
        assert_eq!(r2.access_count, 2);

        let r3 = mem.get("key1").unwrap().unwrap();
        assert_eq!(r3.access_count, 3);
    }

    #[test]
    fn delete_existing_memory() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        mem.save("key1", "value1", MemoryCategory::Fact).unwrap();
        assert!(mem.delete("key1").unwrap());
        assert!(mem.get("key1").unwrap().is_none());
    }

    #[test]
    fn delete_nonexistent_returns_false() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        assert!(!mem.delete("nonexistent").unwrap());
    }

    #[test]
    fn count_memories() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        assert_eq!(mem.count().unwrap(), 0);
        mem.save("a", "va", MemoryCategory::Fact).unwrap();
        assert_eq!(mem.count().unwrap(), 1);
        mem.save("b", "vb", MemoryCategory::Fact).unwrap();
        assert_eq!(mem.count().unwrap(), 2);
    }

    #[test]
    fn evict_removes_least_accessed() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        // Create 5 memories.
        mem.save("a", "va", MemoryCategory::Fact).unwrap();
        mem.save("b", "vb", MemoryCategory::Fact).unwrap();
        mem.save("c", "vc", MemoryCategory::Fact).unwrap();
        mem.save("d", "vd", MemoryCategory::Fact).unwrap();
        mem.save("e", "ve", MemoryCategory::Fact).unwrap();

        // Access some more than others to influence ranking.
        mem.get("c").unwrap(); // c: access_count = 1
        mem.get("c").unwrap(); // c: access_count = 2
        mem.get("e").unwrap(); // e: access_count = 1

        // Evict to keep only 3.
        let evicted = mem.evict(3).unwrap();
        assert_eq!(evicted, 2);
        assert_eq!(mem.count().unwrap(), 3);

        // c and e should survive (highest access_count).
        // The third survivor is one of a, b, d (all access_count 0, most recent updated_at).
        assert!(mem.get("c").unwrap().is_some());
        assert!(mem.get("e").unwrap().is_some());
    }

    #[test]
    fn evict_no_op_when_under_limit() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        mem.save("a", "va", MemoryCategory::Fact).unwrap();

        let evicted = mem.evict(50).unwrap();
        assert_eq!(evicted, 0);
        assert_eq!(mem.count().unwrap(), 1);
    }

    #[test]
    fn top_n_returns_ranked_memories() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        mem.save("low", "vlow", MemoryCategory::Fact).unwrap();
        mem.save("high", "vhigh", MemoryCategory::Fact).unwrap();
        mem.save("mid", "vmid", MemoryCategory::Fact).unwrap();

        // Access "high" 3 times, "mid" 1 time, "low" 0 times.
        mem.get("high").unwrap();
        mem.get("high").unwrap();
        mem.get("high").unwrap();
        mem.get("mid").unwrap();

        let top = mem.top_n(2).unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].key, "high");
        assert_eq!(top[1].key, "mid");
    }

    #[test]
    fn top_n_with_zero_returns_empty() {
        let (_tmp, store) = open_test_store();
        let mem = store.memories();

        mem.save("a", "va", MemoryCategory::Fact).unwrap();

        let top = mem.top_n(0).unwrap();
        assert!(top.is_empty());
    }

    #[test]
    fn category_parsing() {
        assert_eq!(
            "fact".parse::<MemoryCategory>().unwrap(),
            MemoryCategory::Fact
        );
        assert_eq!(
            "FACT".parse::<MemoryCategory>().unwrap(),
            MemoryCategory::Fact
        );
        assert_eq!(
            "Decision".parse::<MemoryCategory>().unwrap(),
            MemoryCategory::Decision
        );
        assert_eq!(
            "preference".parse::<MemoryCategory>().unwrap(),
            MemoryCategory::Preference
        );
        assert!("invalid".parse::<MemoryCategory>().is_err());
        // Also test the convenience parse method.
        assert_eq!(MemoryCategory::parse("fact").unwrap(), MemoryCategory::Fact);
        assert!(MemoryCategory::parse("invalid").is_err());
    }

    #[test]
    fn category_display() {
        assert_eq!(MemoryCategory::Fact.as_str(), "fact");
        assert_eq!(MemoryCategory::Decision.as_str(), "decision");
        assert_eq!(MemoryCategory::Preference.as_str(), "preference");
    }

    #[test]
    fn schema_version_is_3_after_migration() {
        let (_tmp, store) = open_test_store();
        assert_eq!(store.schema_version().unwrap(), 3);
    }
}
