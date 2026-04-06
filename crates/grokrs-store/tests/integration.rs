//! Comprehensive integration tests for grokrs-store.
//!
//! Each test uses `tempfile::tempdir` for isolation — no shared state between
//! tests. Tests exercise the public API surface end-to-end.

use grokrs_store::Store;
use grokrs_store::types::{TranscriptUsage, UsageSummary};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn open_store() -> (tempfile::TempDir, Store) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path()).unwrap();
    (tmp, store)
}

// ---------------------------------------------------------------------------
// Migration tests
// ---------------------------------------------------------------------------

#[test]
fn migration_forward_compat_fresh_db() {
    let (_tmp, store) = open_store();
    assert_eq!(store.schema_version().unwrap(), 3);
}

#[test]
fn migration_reopen_is_noop() {
    let tmp = tempfile::tempdir().unwrap();

    {
        let store = Store::open(tmp.path()).unwrap();
        assert_eq!(store.schema_version().unwrap(), 3);
        store.close().unwrap();
    }

    {
        let store = Store::open(tmp.path()).unwrap();
        assert_eq!(store.schema_version().unwrap(), 3);
    }
}

// ---------------------------------------------------------------------------
// Session lifecycle tests
// ---------------------------------------------------------------------------

#[test]
fn session_create_and_get() {
    let (_tmp, store) = open_store();
    store.sessions().create("sess-1", "Untrusted").unwrap();

    let record = store
        .sessions()
        .get("sess-1")
        .unwrap()
        .expect("session should exist");
    assert_eq!(record.id, "sess-1");
    assert_eq!(record.trust_level, "Untrusted");
    assert_eq!(record.state, "Created");
    assert!(record.created_at.ends_with('Z'));
    assert!(record.updated_at.ends_with('Z'));
}

#[test]
fn session_get_nonexistent_returns_none() {
    let (_tmp, store) = open_store();
    assert!(store.sessions().get("nope").unwrap().is_none());
}

#[test]
fn session_duplicate_id_returns_error() {
    let (_tmp, store) = open_store();
    store.sessions().create("dup", "Untrusted").unwrap();
    let result = store.sessions().create("dup", "AdminTrusted");
    assert!(result.is_err());
}

#[test]
fn session_transition_updates_state() {
    let (_tmp, store) = open_store();
    store
        .sessions()
        .create("sess-t", "InteractiveTrusted")
        .unwrap();

    store.sessions().transition("sess-t", "Ready").unwrap();
    let rec = store.sessions().get("sess-t").unwrap().unwrap();
    assert_eq!(rec.state, "Ready");
}

#[test]
fn session_transition_nonexistent_returns_error() {
    let (_tmp, store) = open_store();
    let result = store.sessions().transition("ghost", "Ready");
    assert!(result.is_err());
}

#[test]
fn session_full_lifecycle() {
    let (_tmp, store) = open_store();
    store.sessions().create("lc", "Untrusted").unwrap();

    for state in &["Ready", "RunningTurn", "WaitingApproval", "Closed"] {
        store.sessions().transition("lc", state).unwrap();
        let rec = store.sessions().get("lc").unwrap().unwrap();
        assert_eq!(rec.state, *state);
    }
}

#[test]
fn session_failed_state_preserves_message() {
    let (_tmp, store) = open_store();
    store.sessions().create("fail", "Untrusted").unwrap();
    store
        .sessions()
        .transition("fail", "Failed: connection timeout")
        .unwrap();

    let rec = store.sessions().get("fail").unwrap().unwrap();
    assert_eq!(rec.state, "Failed: connection timeout");
}

#[test]
fn session_list_active_excludes_closed_and_failed() {
    let (_tmp, store) = open_store();

    store.sessions().create("a1", "Untrusted").unwrap();
    store.sessions().transition("a1", "Ready").unwrap();

    store.sessions().create("a2", "Untrusted").unwrap();
    store.sessions().transition("a2", "Closed").unwrap();

    store.sessions().create("a3", "Untrusted").unwrap();
    store.sessions().transition("a3", "Failed: boom").unwrap();

    store.sessions().create("a4", "AdminTrusted").unwrap();
    store.sessions().transition("a4", "RunningTurn").unwrap();

    let active = store.sessions().list_active().unwrap();
    let ids: Vec<&str> = active.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"a1"));
    assert!(!ids.contains(&"a2")); // Closed
    assert!(!ids.contains(&"a3")); // Failed
    assert!(ids.contains(&"a4"));
}

#[test]
fn session_list_active_empty_when_all_closed() {
    let (_tmp, store) = open_store();

    store.sessions().create("c1", "Untrusted").unwrap();
    store.sessions().transition("c1", "Closed").unwrap();

    let active = store.sessions().list_active().unwrap();
    assert!(active.is_empty());
}

#[test]
fn session_trust_level_roundtrips() {
    let (_tmp, store) = open_store();

    for level in &["Untrusted", "InteractiveTrusted", "AdminTrusted"] {
        let id = format!("tl-{level}");
        store.sessions().create(&id, level).unwrap();
        let rec = store.sessions().get(&id).unwrap().unwrap();
        assert_eq!(rec.trust_level, *level);
    }
}

// ---------------------------------------------------------------------------
// Transcript tests
// ---------------------------------------------------------------------------

#[test]
fn transcript_log_request_returns_positive_id() {
    let (_tmp, store) = open_store();
    store.sessions().create("ts", "Untrusted").unwrap();

    let tid = store
        .transcripts()
        .log_request("ts", "/v1/chat", "POST", Some(r#"{"prompt":"hi"}"#))
        .unwrap();
    assert!(tid > 0);
}

#[test]
fn transcript_log_request_sets_request_at() {
    let (_tmp, store) = open_store();
    store.sessions().create("ts2", "Untrusted").unwrap();

    let tid = store
        .transcripts()
        .log_request("ts2", "/v1/chat", "POST", None)
        .unwrap();

    let records = store.transcripts().list_by_session("ts2").unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, tid);
    assert!(records[0].request_at.ends_with('Z'));
}

#[test]
fn transcript_foreign_key_enforced() {
    let (_tmp, store) = open_store();
    let result = store
        .transcripts()
        .log_request("nonexistent", "/v1/chat", "POST", None);
    assert!(result.is_err());
}

#[test]
fn transcript_log_response_updates_record() {
    let (_tmp, store) = open_store();
    store.sessions().create("tr", "Untrusted").unwrap();

    let tid = store
        .transcripts()
        .log_request("tr", "/v1/chat", "POST", Some("{}"))
        .unwrap();

    let usage = TranscriptUsage {
        cost_in_usd_ticks: Some(42),
        input_tokens: Some(100),
        output_tokens: Some(200),
        reasoning_tokens: Some(50),
    };
    store
        .transcripts()
        .log_response(tid, 200, Some(r#"{"text":"hello"}"#), &usage, None)
        .unwrap();

    let records = store.transcripts().list_by_session("tr").unwrap();
    assert_eq!(records.len(), 1);
    let rec = &records[0];
    assert_eq!(rec.status_code, Some(200));
    assert_eq!(rec.cost_in_usd_ticks, Some(42));
    assert_eq!(rec.input_tokens, Some(100));
    assert_eq!(rec.output_tokens, Some(200));
    assert_eq!(rec.reasoning_tokens, Some(50));
    assert!(rec.response_at.is_some());
    assert_eq!(rec.response_body.as_deref(), Some(r#"{"text":"hello"}"#));
}

#[test]
fn transcript_log_error_sets_error_field() {
    let (_tmp, store) = open_store();
    store.sessions().create("te", "Untrusted").unwrap();

    let tid = store
        .transcripts()
        .log_request("te", "/v1/chat", "POST", None)
        .unwrap();

    store
        .transcripts()
        .log_error(tid, "connection refused")
        .unwrap();

    let records = store.transcripts().list_by_session("te").unwrap();
    assert_eq!(records[0].error.as_deref(), Some("connection refused"));
}

#[test]
fn transcript_log_error_on_completed_transcript() {
    let (_tmp, store) = open_store();
    store.sessions().create("tec", "Untrusted").unwrap();

    let tid = store
        .transcripts()
        .log_request("tec", "/v1/chat", "POST", None)
        .unwrap();

    store
        .transcripts()
        .log_response(tid, 200, None, &TranscriptUsage::default(), None)
        .unwrap();

    // Overwriting with error after completion should still succeed.
    store.transcripts().log_error(tid, "late error").unwrap();

    let records = store.transcripts().list_by_session("tec").unwrap();
    assert_eq!(records[0].error.as_deref(), Some("late error"));
}

#[test]
fn transcript_list_by_session_ordered_by_request_at() {
    let (_tmp, store) = open_store();
    store.sessions().create("to", "Untrusted").unwrap();

    let _t1 = store
        .transcripts()
        .log_request("to", "/v1/a", "GET", None)
        .unwrap();
    let _t2 = store
        .transcripts()
        .log_request("to", "/v1/b", "POST", None)
        .unwrap();

    let records = store.transcripts().list_by_session("to").unwrap();
    assert_eq!(records.len(), 2);
    assert!(records[0].request_at <= records[1].request_at);
}

#[test]
fn transcript_list_by_session_empty_for_no_transcripts() {
    let (_tmp, store) = open_store();
    store.sessions().create("empty", "Untrusted").unwrap();

    let records = store.transcripts().list_by_session("empty").unwrap();
    assert!(records.is_empty());
}

#[test]
fn transcript_log_response_nonexistent_id_returns_error() {
    let (_tmp, store) = open_store();
    let result =
        store
            .transcripts()
            .log_response(9999, 200, None, &TranscriptUsage::default(), None);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Usage aggregation tests
// ---------------------------------------------------------------------------

#[test]
fn usage_session_totals_with_multiple_transcripts() {
    let (_tmp, store) = open_store();
    store.sessions().create("us", "Untrusted").unwrap();

    for i in 0..3 {
        let tid = store
            .transcripts()
            .log_request("us", "/v1/chat", "POST", None)
            .unwrap();
        store
            .transcripts()
            .log_response(
                tid,
                200,
                None,
                &TranscriptUsage {
                    cost_in_usd_ticks: Some(10 * (i as i64 + 1)),
                    input_tokens: Some(100),
                    output_tokens: Some(200),
                    reasoning_tokens: Some(50),
                },
                None,
            )
            .unwrap();
    }

    let totals = store.usage().session_totals("us").unwrap();
    assert_eq!(totals.total_cost_ticks, 60); // 10 + 20 + 30
    assert_eq!(totals.total_input_tokens, 300);
    assert_eq!(totals.total_output_tokens, 600);
    assert_eq!(totals.total_reasoning_tokens, 150);
    assert_eq!(totals.request_count, 3);
}

#[test]
fn usage_session_totals_zero_for_no_transcripts() {
    let (_tmp, store) = open_store();
    store.sessions().create("nouse", "Untrusted").unwrap();

    let totals = store.usage().session_totals("nouse").unwrap();
    assert_eq!(totals, UsageSummary::default());
}

#[test]
fn usage_session_totals_coalesces_null_tokens() {
    let (_tmp, store) = open_store();
    store.sessions().create("null", "Untrusted").unwrap();

    let tid = store
        .transcripts()
        .log_request("null", "/v1/chat", "POST", None)
        .unwrap();

    // log_response with all None usage
    store
        .transcripts()
        .log_response(tid, 200, None, &TranscriptUsage::default(), None)
        .unwrap();

    let totals = store.usage().session_totals("null").unwrap();
    assert_eq!(totals.total_cost_ticks, 0);
    assert_eq!(totals.total_input_tokens, 0);
    assert_eq!(totals.request_count, 1);
}

#[test]
fn usage_all_totals_across_sessions() {
    let (_tmp, store) = open_store();

    store.sessions().create("all1", "Untrusted").unwrap();
    store.sessions().create("all2", "Untrusted").unwrap();

    let t1 = store
        .transcripts()
        .log_request("all1", "/v1/chat", "POST", None)
        .unwrap();
    store
        .transcripts()
        .log_response(
            t1,
            200,
            None,
            &TranscriptUsage {
                cost_in_usd_ticks: Some(100),
                input_tokens: Some(50),
                output_tokens: Some(75),
                reasoning_tokens: Some(25),
            },
            None,
        )
        .unwrap();

    let t2 = store
        .transcripts()
        .log_request("all2", "/v1/chat", "POST", None)
        .unwrap();
    store
        .transcripts()
        .log_response(
            t2,
            200,
            None,
            &TranscriptUsage {
                cost_in_usd_ticks: Some(200),
                input_tokens: Some(150),
                output_tokens: Some(225),
                reasoning_tokens: Some(75),
            },
            None,
        )
        .unwrap();

    let totals = store.usage().all_totals().unwrap();
    assert_eq!(totals.total_cost_ticks, 300);
    assert_eq!(totals.total_input_tokens, 200);
    assert_eq!(totals.total_output_tokens, 300);
    assert_eq!(totals.total_reasoning_tokens, 100);
    assert_eq!(totals.request_count, 2);
}

#[test]
fn usage_all_totals_zero_when_empty() {
    let (_tmp, store) = open_store();
    let totals = store.usage().all_totals().unwrap();
    assert_eq!(totals, UsageSummary::default());
}

// ---------------------------------------------------------------------------
// WAL concurrent read test
// ---------------------------------------------------------------------------

#[test]
fn wal_concurrent_read_during_write() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    // First, create the database and seed it with a session so migrations are
    // already applied before the concurrent test begins.
    let tmp = tempfile::tempdir().unwrap();
    {
        let store = Store::open(tmp.path()).unwrap();
        store
            .sessions()
            .create("pre-existing", "Untrusted")
            .unwrap();
        store.close().unwrap();
    }

    let root = tmp.path().to_path_buf();

    // Three-phase barrier:
    //   Phase 1: writer begins transaction and inserts, signals reader.
    //   Phase 2: reader reads while writer's transaction is still open, signals writer.
    //   Phase 3: writer commits, both threads complete.
    let barrier = Arc::new(Barrier::new(2));

    // Writer thread: open a raw connection, BEGIN an explicit transaction,
    // insert data, signal the reader, wait for reader to finish, then COMMIT.
    let writer_root = root.clone();
    let writer_barrier = Arc::clone(&barrier);
    let writer = thread::spawn(move || {
        let db_path = writer_root.join(".grokrs/state.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "busy_timeout", 5000).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();

        // Begin an explicit transaction and insert uncommitted data.
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        conn.execute(
            "INSERT INTO sessions (id, trust_level, state, created_at, updated_at) \
             VALUES ('uncommitted', 'Untrusted', 'Created', '2026-04-05T00:00:00Z', '2026-04-05T00:00:00Z')",
            [],
        )
        .unwrap();

        // Phase 1: signal the reader that the write transaction is open.
        writer_barrier.wait();

        // Phase 2: wait for reader to confirm it read successfully.
        writer_barrier.wait();

        // Now commit so the thread can clean up.
        conn.execute_batch("COMMIT").unwrap();
    });

    // Reader thread: wait for writer's transaction to be open, then read.
    let reader_root = root.clone();
    let reader_barrier = Arc::clone(&barrier);
    let reader = thread::spawn(move || {
        // Phase 1: wait until writer has an open transaction with uncommitted data.
        reader_barrier.wait();

        // Open a separate raw Connection (not Store::open, which runs
        // migrations and would try to acquire a write lock that conflicts
        // with the writer's BEGIN IMMEDIATE). WAL mode allows concurrent
        // readers to see the pre-write snapshot.
        let db_path = reader_root.join(".grokrs/state.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "busy_timeout", 5000).unwrap();

        let mut stmt = conn
            .prepare("SELECT id FROM sessions WHERE state NOT IN ('Closed', 'Failed')")
            .unwrap();
        let ids: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        // The reader should see the pre-existing session but NOT the
        // uncommitted "uncommitted" session (it's in an open transaction).
        let ids: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
        assert!(
            ids.contains(&"pre-existing"),
            "reader should see committed data"
        );
        assert!(
            !ids.contains(&"uncommitted"),
            "reader must NOT see uncommitted data from an open transaction"
        );

        // Phase 2: signal writer that read completed successfully.
        reader_barrier.wait();
    });

    writer.join().unwrap();
    reader.join().unwrap();

    // After both threads complete, verify the committed data is visible.
    {
        let store = Store::open(tmp.path()).unwrap();
        let rec = store.sessions().get("uncommitted").unwrap();
        assert!(
            rec.is_some(),
            "uncommitted session should be visible after commit"
        );
    }
}

// ---------------------------------------------------------------------------
// Crash recovery tests
// ---------------------------------------------------------------------------

#[test]
fn crash_recovery_committed_data_survives_drop_without_close() {
    let tmp = tempfile::tempdir().unwrap();

    // Open, insert data (auto-committed), drop without calling close().
    {
        let store = Store::open(tmp.path()).unwrap();
        store.sessions().create("crash", "Untrusted").unwrap();
        // Drop without close -- no WAL checkpoint, but auto-committed data
        // is durable in the WAL and recovered on next open.
        drop(store);
    }

    // Reopen and verify committed data is present.
    {
        let store = Store::open(tmp.path()).unwrap();
        let rec = store.sessions().get("crash").unwrap();
        assert!(
            rec.is_some(),
            "committed session should persist after drop without close (WAL recovery)"
        );
        assert_eq!(rec.unwrap().state, "Created");
    }
}

#[test]
fn crash_recovery_uncommitted_transaction_is_rolled_back() {
    let tmp = tempfile::tempdir().unwrap();

    // First, create the database with migrations applied and one committed session.
    {
        let store = Store::open(tmp.path()).unwrap();
        store.sessions().create("committed", "Untrusted").unwrap();
        store.close().unwrap();
    }

    // Open a raw connection, begin an explicit transaction, insert data,
    // then DROP the connection WITHOUT committing. This simulates a crash
    // mid-transaction.
    {
        let db_path = tmp.path().join(".grokrs/state.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();

        conn.execute_batch("BEGIN").unwrap();
        conn.execute(
            "INSERT INTO sessions (id, trust_level, state, created_at, updated_at) \
             VALUES ('uncommitted', 'Untrusted', 'Created', '2026-04-05T00:00:00Z', '2026-04-05T00:00:00Z')",
            [],
        )
        .unwrap();

        // Verify the uncommitted row is visible within this transaction.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = 'uncommitted'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "uncommitted row should be visible inside the transaction"
        );

        // DROP without COMMIT -- simulates crash / incomplete transaction.
        drop(conn);
    }

    // Reopen through Store and verify the uncommitted data was rolled back.
    {
        let store = Store::open(tmp.path()).unwrap();

        // The committed session should still be present.
        let committed = store.sessions().get("committed").unwrap();
        assert!(
            committed.is_some(),
            "committed session should survive crash recovery"
        );

        // The uncommitted session should NOT be present -- rolled back.
        let uncommitted = store.sessions().get("uncommitted").unwrap();
        assert!(
            uncommitted.is_none(),
            "uncommitted session must be rolled back after crash (connection drop without commit)"
        );
    }
}

// ---------------------------------------------------------------------------
// Path validation tests
// ---------------------------------------------------------------------------

#[test]
fn store_open_rejects_parent_traversal() {
    let tmp = tempfile::tempdir().unwrap();
    let result = Store::open_with_path(tmp.path(), "../escape/state.db");
    assert!(result.is_err());
}

#[test]
fn store_open_rejects_absolute_path() {
    let tmp = tempfile::tempdir().unwrap();
    let result = Store::open_with_path(tmp.path(), "/tmp/state.db");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// File permissions test (Unix only)
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn file_permissions_are_0600() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path()).unwrap();
    let meta = std::fs::metadata(store.db_path()).unwrap();
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "expected 0600, got {mode:o}");
}

// ---------------------------------------------------------------------------
// Negative token count validation tests
// ---------------------------------------------------------------------------

#[test]
fn negative_token_values_do_not_silently_wrap() {
    let (_tmp, store) = open_store();
    store.sessions().create("neg-tok", "Untrusted").unwrap();

    let tid = store
        .transcripts()
        .log_request("neg-tok", "/v1/chat", "POST", None)
        .unwrap();

    // Write a valid response first, then tamper with the data via raw SQL to
    // inject a negative token count (simulating data corruption or a bug in
    // an older version that accepted signed values).
    store
        .transcripts()
        .log_response(tid, 200, None, &TranscriptUsage::default(), None)
        .unwrap();

    store
        .conn_for_testing()
        .execute(
            "UPDATE transcripts SET input_tokens = -42 WHERE id = ?1",
            rusqlite::params![tid],
        )
        .unwrap();

    // Reading the tampered transcript should produce an error, not wrap -42
    // to u64::MAX - 41.
    let result = store.transcripts().list_by_session("neg-tok");
    assert!(
        result.is_err(),
        "negative token count should produce an error, not silently wrap"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("negative token count"),
        "error should mention 'negative token count', got: {err_msg}"
    );
}

#[test]
fn negative_token_values_rejected_in_usage_aggregation() {
    let (_tmp, store) = open_store();
    store.sessions().create("neg-agg", "Untrusted").unwrap();

    let tid = store
        .transcripts()
        .log_request("neg-agg", "/v1/chat", "POST", None)
        .unwrap();

    store
        .transcripts()
        .log_response(tid, 200, None, &TranscriptUsage::default(), None)
        .unwrap();

    // Tamper via raw SQL to inject a negative sum.
    store
        .conn_for_testing()
        .execute(
            "UPDATE transcripts SET output_tokens = -100 WHERE id = ?1",
            rusqlite::params![tid],
        )
        .unwrap();

    // Both session_totals and all_totals should detect the negative sum.
    let session_result = store.usage().session_totals("neg-agg");
    assert!(
        session_result.is_err(),
        "session_totals should reject negative token sums"
    );

    let all_result = store.usage().all_totals();
    assert!(
        all_result.is_err(),
        "all_totals should reject negative token sums"
    );
}

// ---------------------------------------------------------------------------
// Future extension table tests (U20: schema only, no Rust API)
// ---------------------------------------------------------------------------

#[test]
fn approvals_table_exists_with_correct_columns() {
    let (_tmp, store) = open_store();
    store.sessions().create("ap-sess", "Untrusted").unwrap();

    // Insert via raw SQL to verify schema.
    store
        .conn_for_testing()
        .execute(
            "INSERT INTO approvals (session_id, effect, decision, decided_at, decided_by) \
             VALUES ('ap-sess', 'FsRead', 'Allow', '2026-04-05T00:00:00Z', 'operator')",
            [],
        )
        .unwrap();

    let count: i64 = store
        .conn_for_testing()
        .query_row("SELECT COUNT(*) FROM approvals", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn evidence_table_exists_with_correct_columns() {
    let (_tmp, store) = open_store();
    store.sessions().create("ev-sess", "Untrusted").unwrap();

    store
        .conn_for_testing()
        .execute(
            "INSERT INTO evidence (session_id, kind, payload, created_at, expires_at) \
             VALUES ('ev-sess', 'test-pass', '{\"suite\":\"unit\"}', '2026-04-05T00:00:00Z', '2026-04-06T00:00:00Z')",
            [],
        )
        .unwrap();

    let count: i64 = store
        .conn_for_testing()
        .query_row("SELECT COUNT(*) FROM evidence", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn approvals_foreign_key_enforced() {
    let (_tmp, store) = open_store();

    let result = store.conn_for_testing().execute(
        "INSERT INTO approvals (session_id, effect, decision, decided_at) \
         VALUES ('nonexistent', 'FsRead', 'Allow', '2026-04-05T00:00:00Z')",
        [],
    );
    assert!(
        result.is_err(),
        "FK violation should be enforced on approvals"
    );
}

#[test]
fn evidence_foreign_key_enforced() {
    let (_tmp, store) = open_store();

    let result = store.conn_for_testing().execute(
        "INSERT INTO evidence (session_id, kind, payload, created_at) \
         VALUES ('nonexistent', 'test', '{}', '2026-04-05T00:00:00Z')",
        [],
    );
    assert!(
        result.is_err(),
        "FK violation should be enforced on evidence"
    );
}

// ---------------------------------------------------------------------------
// V2 migration tests
// ---------------------------------------------------------------------------

#[test]
fn v2_migration_applies_on_fresh_db() {
    let (_tmp, store) = open_store();
    assert_eq!(store.schema_version().unwrap(), 3);

    // Verify transcripts table has response_id column.
    store.sessions().create("v2-fresh", "Untrusted").unwrap();
    let tid = store
        .transcripts()
        .log_request("v2-fresh", "/v1/chat", "POST", None)
        .unwrap();
    store
        .transcripts()
        .log_response(
            tid,
            200,
            None,
            &TranscriptUsage::default(),
            Some("resp_abc123"),
        )
        .unwrap();

    let resp_id = store
        .transcripts()
        .get_last_response_id("v2-fresh")
        .unwrap();
    assert_eq!(resp_id.as_deref(), Some("resp_abc123"));
}

#[test]
fn v2_migration_applies_on_existing_v1_db() {
    let tmp = tempfile::tempdir().unwrap();

    // Simulate a V1 database by running only V1 migration manually.
    {
        let db_path = tmp.path().join(".grokrs/state.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute_batch(grokrs_store::migrations::v001::SQL)
            .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();

        // Insert V1 data: a session and a transcript.
        conn.execute(
            "INSERT INTO sessions (id, trust_level, state, created_at, updated_at) \
             VALUES ('old-sess', 'Untrusted', 'Ready', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transcripts (session_id, request_at, endpoint, method) \
             VALUES ('old-sess', '2026-01-01T00:00:01Z', '/v1/chat', 'POST')",
            [],
        )
        .unwrap();
    }

    // Open via Store which should run V2 migration.
    let store = Store::open(tmp.path()).unwrap();
    assert_eq!(store.schema_version().unwrap(), 3);

    // Verify old data survived the migration.
    let sess = store.sessions().get("old-sess").unwrap();
    assert!(sess.is_some(), "V1 session should survive V2 migration");

    let transcripts = store.transcripts().list_by_session("old-sess").unwrap();
    assert_eq!(
        transcripts.len(),
        1,
        "V1 transcript should survive V2 migration"
    );

    // Verify response_id column exists and is NULL for migrated data.
    assert!(transcripts[0].response_id.is_none());

    // Verify ON DELETE CASCADE works on the migrated DB.
    store
        .conn_for_testing()
        .execute("DELETE FROM sessions WHERE id = 'old-sess'", [])
        .unwrap();
    let count: i64 = store
        .conn_for_testing()
        .query_row(
            "SELECT COUNT(*) FROM transcripts WHERE session_id = 'old-sess'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 0,
        "CASCADE should delete transcripts when session is deleted"
    );
}

#[test]
fn v2_cascade_deletes_transcripts() {
    let (_tmp, store) = open_store();
    store.sessions().create("cas-sess", "Untrusted").unwrap();
    store
        .transcripts()
        .log_request("cas-sess", "/v1/chat", "POST", None)
        .unwrap();
    store
        .transcripts()
        .log_request("cas-sess", "/v1/chat", "POST", None)
        .unwrap();

    // Verify transcripts exist.
    let count = store.sessions().count_transcripts("cas-sess").unwrap();
    assert_eq!(count, 2);

    // Delete the session directly via SQL; CASCADE should remove transcripts.
    store
        .conn_for_testing()
        .execute("DELETE FROM sessions WHERE id = 'cas-sess'", [])
        .unwrap();

    let transcript_count: i64 = store
        .conn_for_testing()
        .query_row(
            "SELECT COUNT(*) FROM transcripts WHERE session_id = 'cas-sess'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(transcript_count, 0, "CASCADE should delete transcripts");
}

#[test]
fn v2_cascade_deletes_approvals_and_evidence() {
    let (_tmp, store) = open_store();
    store.sessions().create("cas2", "Untrusted").unwrap();

    store
        .conn_for_testing()
        .execute(
            "INSERT INTO approvals (session_id, effect, decision, decided_at) \
             VALUES ('cas2', 'FsRead', 'Allow', '2026-04-05T00:00:00Z')",
            [],
        )
        .unwrap();
    store
        .conn_for_testing()
        .execute(
            "INSERT INTO evidence (session_id, kind, payload, created_at) \
             VALUES ('cas2', 'test', '{}', '2026-04-05T00:00:00Z')",
            [],
        )
        .unwrap();

    store
        .conn_for_testing()
        .execute("DELETE FROM sessions WHERE id = 'cas2'", [])
        .unwrap();

    let approvals: i64 = store
        .conn_for_testing()
        .query_row(
            "SELECT COUNT(*) FROM approvals WHERE session_id = 'cas2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let evidence: i64 = store
        .conn_for_testing()
        .query_row(
            "SELECT COUNT(*) FROM evidence WHERE session_id = 'cas2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(approvals, 0, "CASCADE should delete approvals");
    assert_eq!(evidence, 0, "CASCADE should delete evidence");
}

// ---------------------------------------------------------------------------
// SessionRepo extension tests (U23)
// ---------------------------------------------------------------------------

#[test]
fn list_all_returns_sessions_ordered_by_updated_at_desc() {
    let (_tmp, store) = open_store();

    // Create sessions with different updated_at by transitioning.
    store.sessions().create("la-1", "Untrusted").unwrap();
    store.sessions().create("la-2", "Untrusted").unwrap();
    store.sessions().transition("la-1", "Ready").unwrap(); // la-1 updated more recently

    let all = store.sessions().list_all(None).unwrap();
    assert_eq!(all.len(), 2);
    // la-1 was updated after la-2, so it should be first.
    assert_eq!(all[0].id, "la-1");
    assert_eq!(all[1].id, "la-2");
}

#[test]
fn list_all_with_limit() {
    let (_tmp, store) = open_store();
    store.sessions().create("lim-1", "Untrusted").unwrap();
    store.sessions().create("lim-2", "Untrusted").unwrap();
    store.sessions().create("lim-3", "Untrusted").unwrap();

    let limited = store.sessions().list_all(Some(2)).unwrap();
    assert_eq!(limited.len(), 2);
}

#[test]
fn list_all_no_limit_returns_all() {
    let (_tmp, store) = open_store();
    for i in 0..5 {
        store
            .sessions()
            .create(&format!("all-{i}"), "Untrusted")
            .unwrap();
    }
    let all = store.sessions().list_all(None).unwrap();
    assert_eq!(all.len(), 5);
}

#[test]
fn list_by_state_filters_correctly() {
    let (_tmp, store) = open_store();
    store.sessions().create("bs-1", "Untrusted").unwrap();
    store.sessions().transition("bs-1", "Ready").unwrap();

    store.sessions().create("bs-2", "Untrusted").unwrap();
    store.sessions().transition("bs-2", "Closed").unwrap();

    store.sessions().create("bs-3", "Untrusted").unwrap();
    store.sessions().transition("bs-3", "Ready").unwrap();

    let ready = store.sessions().list_by_state("Ready").unwrap();
    assert_eq!(ready.len(), 2);
    assert!(ready.iter().all(|s| s.state == "Ready"));

    let closed = store.sessions().list_by_state("Closed").unwrap();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].id, "bs-2");
}

#[test]
fn list_by_state_nonexistent_returns_empty() {
    let (_tmp, store) = open_store();
    store.sessions().create("nope", "Untrusted").unwrap();
    let result = store.sessions().list_by_state("SomeInventedState").unwrap();
    assert!(result.is_empty());
}

#[test]
fn find_by_prefix_matches_correctly() {
    let (_tmp, store) = open_store();
    store.sessions().create("abc-001", "Untrusted").unwrap();
    store.sessions().create("abc-002", "Untrusted").unwrap();
    store.sessions().create("xyz-001", "Untrusted").unwrap();

    let found = store.sessions().find_by_prefix("abc").unwrap();
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|s| s.id.starts_with("abc")));
}

#[test]
fn find_by_prefix_no_match_returns_empty() {
    let (_tmp, store) = open_store();
    store.sessions().create("abc", "Untrusted").unwrap();
    let found = store.sessions().find_by_prefix("zzz").unwrap();
    assert!(found.is_empty());
}

#[test]
fn find_by_prefix_escapes_sql_like_special_chars() {
    let (_tmp, store) = open_store();
    // Create sessions with LIKE special characters in their IDs.
    store.sessions().create("100%_done", "Untrusted").unwrap();
    store.sessions().create("100abc", "Untrusted").unwrap();
    store.sessions().create("100%_other", "Untrusted").unwrap();

    // Searching for "100%" should match "100%_done" and "100%_other" but NOT "100abc".
    let found = store.sessions().find_by_prefix("100%").unwrap();
    assert_eq!(found.len(), 2, "should match only IDs starting with '100%'");
    assert!(found.iter().all(|s| s.id.starts_with("100%")));

    // Searching for "100%_" should match both "100%_done" and "100%_other".
    let found2 = store.sessions().find_by_prefix("100%_").unwrap();
    assert_eq!(found2.len(), 2);

    // Searching for "100%_d" should match only "100%_done".
    let found3 = store.sessions().find_by_prefix("100%_d").unwrap();
    assert_eq!(found3.len(), 1);
    assert_eq!(found3[0].id, "100%_done");
}

#[test]
fn delete_old_cascades_to_transcripts() {
    let (_tmp, store) = open_store();

    // Create a session with old timestamps.
    store
        .conn_for_testing()
        .execute(
            "INSERT INTO sessions (id, trust_level, state, created_at, updated_at) \
             VALUES ('old', 'Untrusted', 'Closed', '2020-01-01T00:00:00Z', '2020-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
    store
        .conn_for_testing()
        .execute(
            "INSERT INTO transcripts (session_id, request_at, endpoint, method) \
             VALUES ('old', '2020-01-01T00:00:01Z', '/v1/chat', 'POST')",
            [],
        )
        .unwrap();

    // Create a recent session that should NOT be deleted.
    store.sessions().create("recent", "Untrusted").unwrap();
    store
        .transcripts()
        .log_request("recent", "/v1/chat", "POST", None)
        .unwrap();

    // Delete sessions older than 2025.
    let deleted = store.sessions().delete_old("2025-01-01T00:00:00Z").unwrap();
    assert_eq!(deleted, 1);

    // Old session and its transcripts should be gone.
    assert!(store.sessions().get("old").unwrap().is_none());
    let old_transcripts: i64 = store
        .conn_for_testing()
        .query_row(
            "SELECT COUNT(*) FROM transcripts WHERE session_id = 'old'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(old_transcripts, 0, "CASCADE should delete old transcripts");

    // Recent session should still exist.
    assert!(store.sessions().get("recent").unwrap().is_some());
    let recent_transcripts = store.sessions().count_transcripts("recent").unwrap();
    assert_eq!(recent_transcripts, 1);
}

#[test]
fn delete_old_returns_zero_when_nothing_to_delete() {
    let (_tmp, store) = open_store();
    store.sessions().create("s1", "Untrusted").unwrap();
    // All sessions are recent, threshold is far in the past.
    let deleted = store.sessions().delete_old("2000-01-01T00:00:00Z").unwrap();
    assert_eq!(deleted, 0);
}

#[test]
fn count_transcripts_returns_correct_count() {
    let (_tmp, store) = open_store();
    store.sessions().create("ct", "Untrusted").unwrap();

    assert_eq!(store.sessions().count_transcripts("ct").unwrap(), 0);

    store
        .transcripts()
        .log_request("ct", "/v1/a", "GET", None)
        .unwrap();
    store
        .transcripts()
        .log_request("ct", "/v1/b", "POST", None)
        .unwrap();

    assert_eq!(store.sessions().count_transcripts("ct").unwrap(), 2);
}

// ---------------------------------------------------------------------------
// TranscriptRepo extension tests (U23)
// ---------------------------------------------------------------------------

#[test]
fn response_id_stored_and_retrieved() {
    let (_tmp, store) = open_store();
    store.sessions().create("rid", "Untrusted").unwrap();

    let tid = store
        .transcripts()
        .log_request("rid", "/v1/chat", "POST", None)
        .unwrap();

    store
        .transcripts()
        .log_response(
            tid,
            200,
            Some("{}"),
            &TranscriptUsage::default(),
            Some("resp_xyz789"),
        )
        .unwrap();

    // Verify via list_by_session.
    let records = store.transcripts().list_by_session("rid").unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].response_id.as_deref(), Some("resp_xyz789"));

    // Verify via get_last_response_id.
    let last = store.transcripts().get_last_response_id("rid").unwrap();
    assert_eq!(last.as_deref(), Some("resp_xyz789"));
}

#[test]
fn response_id_none_when_not_set() {
    let (_tmp, store) = open_store();
    store.sessions().create("nrid", "Untrusted").unwrap();

    let tid = store
        .transcripts()
        .log_request("nrid", "/v1/chat", "POST", None)
        .unwrap();
    store
        .transcripts()
        .log_response(tid, 200, None, &TranscriptUsage::default(), None)
        .unwrap();

    let last = store.transcripts().get_last_response_id("nrid").unwrap();
    assert!(last.is_none());
}

#[test]
fn get_last_response_id_returns_most_recent() {
    let (_tmp, store) = open_store();
    store.sessions().create("multi-rid", "Untrusted").unwrap();

    // First request with response_id "first".
    let t1 = store
        .transcripts()
        .log_request("multi-rid", "/v1/chat", "POST", None)
        .unwrap();
    store
        .transcripts()
        .log_response(t1, 200, None, &TranscriptUsage::default(), Some("first"))
        .unwrap();

    // Second request with response_id "second".
    let t2 = store
        .transcripts()
        .log_request("multi-rid", "/v1/chat", "POST", None)
        .unwrap();
    store
        .transcripts()
        .log_response(t2, 200, None, &TranscriptUsage::default(), Some("second"))
        .unwrap();

    let last = store
        .transcripts()
        .get_last_response_id("multi-rid")
        .unwrap();
    assert_eq!(last.as_deref(), Some("second"));
}

#[test]
fn get_last_response_id_empty_session() {
    let (_tmp, store) = open_store();
    store.sessions().create("empty-rid", "Untrusted").unwrap();

    let last = store
        .transcripts()
        .get_last_response_id("empty-rid")
        .unwrap();
    assert!(last.is_none());
}
