//! Integration tests for agent memory CRUD and eviction.
//!
//! These tests exercise the full lifecycle of the memory subsystem:
//! - Create, read, update, delete
//! - Search by key and value substrings
//! - Access counting and ranking
//! - Eviction of least-accessed memories
//! - Category filtering
//! - Interaction between operations (e.g., search affecting eviction order)
//!
//! Each test uses an isolated SQLite database via tempfile::tempdir.

use grokrs_store::memory::MemoryCategory;
use grokrs_store::Store;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn open_store() -> (tempfile::TempDir, Store) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path()).unwrap();
    (tmp, store)
}

// ---------------------------------------------------------------------------
// CRUD lifecycle
// ---------------------------------------------------------------------------

#[test]
fn full_crud_lifecycle() {
    let (_tmp, store) = open_store();
    let mem = store.memories();

    // Create.
    mem.save("project-lang", "Rust", MemoryCategory::Fact)
        .unwrap();
    assert_eq!(mem.count().unwrap(), 1);

    // Read.
    let record = mem.get("project-lang").unwrap().unwrap();
    assert_eq!(record.key, "project-lang");
    assert_eq!(record.value, "Rust");
    assert_eq!(record.category, "fact");
    assert_eq!(record.access_count, 1); // get increments

    // Update (upsert). Access count was 1 after the get above. Upsert does NOT
    // reset access_count — it only updates value, category, and updated_at.
    mem.save("project-lang", "Rust 2024 edition", MemoryCategory::Fact)
        .unwrap();
    let updated = mem.get("project-lang").unwrap().unwrap();
    assert_eq!(updated.value, "Rust 2024 edition");
    assert_eq!(updated.access_count, 2); // 1 from previous get + 1 from this get

    // Delete.
    assert!(mem.delete("project-lang").unwrap());
    assert!(mem.get("project-lang").unwrap().is_none());
    assert_eq!(mem.count().unwrap(), 0);
}

// ---------------------------------------------------------------------------
// Batch operations
// ---------------------------------------------------------------------------

#[test]
fn save_and_list_many_memories() {
    let (_tmp, store) = open_store();
    let mem = store.memories();

    let entries = vec![
        ("rust-edition", "2021", MemoryCategory::Fact),
        ("preferred-editor", "neovim", MemoryCategory::Preference),
        (
            "use-clippy",
            "always run clippy before commit",
            MemoryCategory::Decision,
        ),
        ("test-framework", "built-in #[test]", MemoryCategory::Fact),
        ("indent-style", "4 spaces", MemoryCategory::Preference),
    ];

    for (key, value, category) in &entries {
        mem.save(key, value, *category).unwrap();
    }

    assert_eq!(mem.count().unwrap(), 5);

    let all = mem.list(None).unwrap();
    assert_eq!(all.len(), 5);
}

// ---------------------------------------------------------------------------
// Category filtering
// ---------------------------------------------------------------------------

#[test]
fn list_by_category_filters_correctly() {
    let (_tmp, store) = open_store();
    let mem = store.memories();

    mem.save("lang", "Rust", MemoryCategory::Fact).unwrap();
    mem.save("editor", "neovim", MemoryCategory::Preference)
        .unwrap();
    mem.save("formatter", "rustfmt", MemoryCategory::Decision)
        .unwrap();
    mem.save("edition", "2021", MemoryCategory::Fact).unwrap();
    mem.save("theme", "dark mode", MemoryCategory::Preference)
        .unwrap();

    let facts = mem.list(Some(MemoryCategory::Fact)).unwrap();
    assert_eq!(facts.len(), 2);
    assert!(facts.iter().all(|r| r.category == "fact"));

    let prefs = mem.list(Some(MemoryCategory::Preference)).unwrap();
    assert_eq!(prefs.len(), 2);
    assert!(prefs.iter().all(|r| r.category == "preference"));

    let decisions = mem.list(Some(MemoryCategory::Decision)).unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].key, "formatter");
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

#[test]
fn search_matches_key_and_value() {
    let (_tmp, store) = open_store();
    let mem = store.memories();

    mem.save("rust-toolchain", "stable", MemoryCategory::Fact)
        .unwrap();
    mem.save("python-version", "3.12", MemoryCategory::Fact)
        .unwrap();
    mem.save(
        "build-tool",
        "cargo is the Rust build tool",
        MemoryCategory::Fact,
    )
    .unwrap();

    // Search by key substring.
    let results = mem.search("rust").unwrap();
    assert_eq!(results.len(), 2); // "rust-toolchain" key + "build-tool" value contains "Rust"
    let keys: Vec<&str> = results.iter().map(|r| r.key.as_str()).collect();
    assert!(keys.contains(&"rust-toolchain"));
    assert!(keys.contains(&"build-tool"));
}

#[test]
fn search_no_match_returns_empty() {
    let (_tmp, store) = open_store();
    let mem = store.memories();

    mem.save("key1", "value1", MemoryCategory::Fact).unwrap();

    let results = mem.search("nonexistent-query-xyz").unwrap();
    assert!(results.is_empty());
}

#[test]
fn search_increments_access_count() {
    let (_tmp, store) = open_store();
    let mem = store.memories();

    mem.save("target", "findme", MemoryCategory::Fact).unwrap();

    // First search.
    let r1 = mem.search("findme").unwrap();
    assert_eq!(r1[0].access_count, 1);

    // Second search.
    let r2 = mem.search("findme").unwrap();
    assert_eq!(r2[0].access_count, 2);

    // Third search.
    let r3 = mem.search("findme").unwrap();
    assert_eq!(r3[0].access_count, 3);
}

// ---------------------------------------------------------------------------
// Access counting affects ranking
// ---------------------------------------------------------------------------

#[test]
fn frequently_accessed_memories_rank_higher() {
    let (_tmp, store) = open_store();
    let mem = store.memories();

    mem.save("low", "low-access", MemoryCategory::Fact).unwrap();
    mem.save("mid", "mid-access", MemoryCategory::Fact).unwrap();
    mem.save("high", "high-access", MemoryCategory::Fact)
        .unwrap();

    // Access "high" 5 times, "mid" 2 times, "low" 0 times.
    for _ in 0..5 {
        mem.get("high").unwrap();
    }
    for _ in 0..2 {
        mem.get("mid").unwrap();
    }

    // list returns by access_count DESC.
    let all = mem.list(None).unwrap();
    assert_eq!(all[0].key, "high");
    assert_eq!(all[1].key, "mid");
    assert_eq!(all[2].key, "low");

    // top_n(2) should return the top 2.
    let top = mem.top_n(2).unwrap();
    assert_eq!(top.len(), 2);
    assert_eq!(top[0].key, "high");
    assert_eq!(top[1].key, "mid");
}

// ---------------------------------------------------------------------------
// Eviction
// ---------------------------------------------------------------------------

#[test]
fn eviction_removes_least_accessed_first() {
    let (_tmp, store) = open_store();
    let mem = store.memories();

    // Create 10 memories.
    for i in 0..10 {
        mem.save(
            &format!("mem-{i:02}"),
            &format!("value-{i}"),
            MemoryCategory::Fact,
        )
        .unwrap();
    }
    assert_eq!(mem.count().unwrap(), 10);

    // Access some memories to increase their rank.
    // mem-09: 5 accesses, mem-05: 3 accesses, mem-02: 1 access.
    for _ in 0..5 {
        mem.get("mem-09").unwrap();
    }
    for _ in 0..3 {
        mem.get("mem-05").unwrap();
    }
    mem.get("mem-02").unwrap();

    // Evict to keep only 3.
    let evicted = mem.evict(3).unwrap();
    assert_eq!(evicted, 7);
    assert_eq!(mem.count().unwrap(), 3);

    // The survivors should be the most-accessed ones.
    assert!(mem.get("mem-09").unwrap().is_some());
    assert!(mem.get("mem-05").unwrap().is_some());
    assert!(mem.get("mem-02").unwrap().is_some());
}

#[test]
fn eviction_no_op_when_under_limit() {
    let (_tmp, store) = open_store();
    let mem = store.memories();

    mem.save("a", "va", MemoryCategory::Fact).unwrap();
    mem.save("b", "vb", MemoryCategory::Fact).unwrap();

    let evicted = mem.evict(100).unwrap();
    assert_eq!(evicted, 0);
    assert_eq!(mem.count().unwrap(), 2);
}

#[test]
fn eviction_at_exact_limit_is_no_op() {
    let (_tmp, store) = open_store();
    let mem = store.memories();

    mem.save("a", "va", MemoryCategory::Fact).unwrap();
    mem.save("b", "vb", MemoryCategory::Fact).unwrap();
    mem.save("c", "vc", MemoryCategory::Fact).unwrap();

    let evicted = mem.evict(3).unwrap();
    assert_eq!(evicted, 0);
    assert_eq!(mem.count().unwrap(), 3);
}

#[test]
fn eviction_to_zero_empties_table() {
    let (_tmp, store) = open_store();
    let mem = store.memories();

    mem.save("a", "va", MemoryCategory::Fact).unwrap();
    mem.save("b", "vb", MemoryCategory::Fact).unwrap();

    let evicted = mem.evict(0).unwrap();
    assert_eq!(evicted, 2);
    assert_eq!(mem.count().unwrap(), 0);
}

// ---------------------------------------------------------------------------
// Search affects eviction ordering
// ---------------------------------------------------------------------------

#[test]
fn search_hits_protect_from_eviction() {
    let (_tmp, store) = open_store();
    let mem = store.memories();

    // Create memories.
    mem.save("important", "keep this one", MemoryCategory::Fact)
        .unwrap();
    mem.save("disposable-1", "temporary", MemoryCategory::Fact)
        .unwrap();
    mem.save("disposable-2", "temporary", MemoryCategory::Fact)
        .unwrap();

    // Search for "important" to boost its access count.
    mem.search("important").unwrap();
    mem.search("important").unwrap();

    // Evict to keep 1.
    let evicted = mem.evict(1).unwrap();
    assert_eq!(evicted, 2);

    // "important" should survive.
    assert!(mem.get("important").unwrap().is_some());
    assert_eq!(mem.count().unwrap(), 1);
}

// ---------------------------------------------------------------------------
// Upsert preserves key identity
// ---------------------------------------------------------------------------

#[test]
fn upsert_updates_value_and_category() {
    let (_tmp, store) = open_store();
    let mem = store.memories();

    mem.save("key", "original-value", MemoryCategory::Fact)
        .unwrap();
    let r1 = mem.get("key").unwrap().unwrap();
    assert_eq!(r1.value, "original-value");
    assert_eq!(r1.category, "fact");

    // Upsert with new value and category.
    mem.save("key", "updated-value", MemoryCategory::Decision)
        .unwrap();
    let r2 = mem.get("key").unwrap().unwrap();
    assert_eq!(r2.value, "updated-value");
    assert_eq!(r2.category, "decision");

    // Should still be only 1 memory.
    assert_eq!(mem.count().unwrap(), 1);
}

// ---------------------------------------------------------------------------
// Delete nonexistent
// ---------------------------------------------------------------------------

#[test]
fn delete_nonexistent_returns_false() {
    let (_tmp, store) = open_store();
    let mem = store.memories();
    assert!(!mem.delete("ghost").unwrap());
}

// ---------------------------------------------------------------------------
// top_n edge cases
// ---------------------------------------------------------------------------

#[test]
fn top_n_zero_returns_empty() {
    let (_tmp, store) = open_store();
    let mem = store.memories();
    mem.save("a", "va", MemoryCategory::Fact).unwrap();
    assert!(mem.top_n(0).unwrap().is_empty());
}

#[test]
fn top_n_greater_than_total_returns_all() {
    let (_tmp, store) = open_store();
    let mem = store.memories();
    mem.save("a", "va", MemoryCategory::Fact).unwrap();
    mem.save("b", "vb", MemoryCategory::Fact).unwrap();

    let top = mem.top_n(100).unwrap();
    assert_eq!(top.len(), 2);
}

// ---------------------------------------------------------------------------
// Concurrent-like access patterns (sequential but realistic)
// ---------------------------------------------------------------------------

#[test]
fn realistic_agent_memory_workflow() {
    let (_tmp, store) = open_store();
    let mem = store.memories();

    // Agent learns facts about the project.
    mem.save("project-name", "grokrs", MemoryCategory::Fact)
        .unwrap();
    mem.save("project-lang", "Rust", MemoryCategory::Fact)
        .unwrap();
    mem.save("project-build", "cargo", MemoryCategory::Fact)
        .unwrap();

    // Agent records a decision.
    mem.save(
        "error-handling",
        "use thiserror for library crates, anyhow for binary",
        MemoryCategory::Decision,
    )
    .unwrap();

    // Agent records user preferences.
    mem.save(
        "code-style",
        "prefer explicit error types over unwrap",
        MemoryCategory::Preference,
    )
    .unwrap();
    mem.save(
        "testing",
        "always write integration tests",
        MemoryCategory::Preference,
    )
    .unwrap();

    // Agent recalls project facts (simulating context building).
    let facts = mem.list(Some(MemoryCategory::Fact)).unwrap();
    assert_eq!(facts.len(), 3);

    // Agent searches for relevant memories before executing a task.
    let relevant = mem.search("error").unwrap();
    assert!(!relevant.is_empty());
    assert!(relevant.iter().any(|r| r.key == "error-handling"));
    assert!(relevant.iter().any(|r| r.key == "code-style"));

    // Agent gets top memories for system prompt.
    let _ = mem.get("project-name").unwrap(); // boost access
    let _ = mem.get("project-name").unwrap(); // boost more
    let top = mem.top_n(3).unwrap();
    assert_eq!(top.len(), 3);
    // "project-name" should be first (3 accesses from get calls).
    assert_eq!(top[0].key, "project-name");

    // After many sessions, evict old memories.
    for i in 0..20 {
        mem.save(
            &format!("temp-{i}"),
            &format!("temporary note {i}"),
            MemoryCategory::Fact,
        )
        .unwrap();
    }
    assert_eq!(mem.count().unwrap(), 26);

    // Evict to 10.
    let evicted = mem.evict(10).unwrap();
    assert_eq!(evicted, 16);
    assert_eq!(mem.count().unwrap(), 10);

    // Core memories should survive because they were accessed.
    assert!(mem.get("project-name").unwrap().is_some());
    assert!(mem.get("error-handling").unwrap().is_some());
    assert!(mem.get("code-style").unwrap().is_some());
}

// ---------------------------------------------------------------------------
// Category parsing
// ---------------------------------------------------------------------------

#[test]
fn category_parsing_all_variants() {
    assert_eq!(
        "fact".parse::<MemoryCategory>().unwrap(),
        MemoryCategory::Fact
    );
    assert_eq!(
        "FACT".parse::<MemoryCategory>().unwrap(),
        MemoryCategory::Fact
    );
    assert_eq!(
        "decision".parse::<MemoryCategory>().unwrap(),
        MemoryCategory::Decision
    );
    assert_eq!(
        "Decision".parse::<MemoryCategory>().unwrap(),
        MemoryCategory::Decision
    );
    assert_eq!(
        "preference".parse::<MemoryCategory>().unwrap(),
        MemoryCategory::Preference
    );
    assert_eq!(
        "PREFERENCE".parse::<MemoryCategory>().unwrap(),
        MemoryCategory::Preference
    );

    assert!("invalid".parse::<MemoryCategory>().is_err());
    assert!("".parse::<MemoryCategory>().is_err());
}

#[test]
fn category_display_and_as_str() {
    assert_eq!(MemoryCategory::Fact.to_string(), "fact");
    assert_eq!(MemoryCategory::Decision.to_string(), "decision");
    assert_eq!(MemoryCategory::Preference.to_string(), "preference");

    assert_eq!(MemoryCategory::Fact.as_str(), "fact");
    assert_eq!(MemoryCategory::Decision.as_str(), "decision");
    assert_eq!(MemoryCategory::Preference.as_str(), "preference");
}
