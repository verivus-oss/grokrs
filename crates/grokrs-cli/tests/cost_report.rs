//! Integration tests for cost report aggregation.
//!
//! These tests seed transcript data into an isolated SQLite database and verify
//! that cost aggregation queries (by model, day, session, endpoint) produce
//! correct results. Also tests the summary computation, date filtering, and
//! output formatters.
//!
//! No network access or API keys required.

use grokrs_store::Store;
use grokrs_store::cost::{CostFilter, CostGroupBy, format_table, format_usd};
use grokrs_store::types::TranscriptUsage;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn open_store() -> (tempfile::TempDir, Store) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path()).unwrap();
    (tmp, store)
}

/// Seed a transcript with given parameters.
fn seed_transcript(
    store: &Store,
    session_id: &str,
    endpoint: &str,
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    cost_ticks: i64,
) {
    let request_body = serde_json::json!({ "model": model }).to_string();
    let tid = store
        .transcripts()
        .log_request(session_id, endpoint, "POST", Some(&request_body))
        .unwrap();
    let usage = TranscriptUsage {
        cost_in_usd_ticks: Some(cost_ticks),
        input_tokens: Some(input_tokens),
        output_tokens: Some(output_tokens),
        reasoning_tokens: Some(reasoning_tokens),
    };
    store
        .transcripts()
        .log_response(tid, 200, None, &usage, None)
        .unwrap();
}

// ---------------------------------------------------------------------------
// Tests: Aggregation by model
// ---------------------------------------------------------------------------

#[test]
fn aggregate_by_model_single_model() {
    let (_tmp, store) = open_store();
    store.sessions().create("s1", "Untrusted").unwrap();

    seed_transcript(&store, "s1", "/v1/responses", "grok-3", 100, 200, 0, 50_000);
    seed_transcript(
        &store,
        "s1",
        "/v1/responses",
        "grok-3",
        150,
        250,
        50,
        75_000,
    );

    let rows = store
        .cost()
        .aggregate(CostGroupBy::Model, &CostFilter::default())
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].group, "grok-3");
    assert_eq!(rows[0].requests, 2);
    assert_eq!(rows[0].input_tokens, 250);
    assert_eq!(rows[0].output_tokens, 450);
    assert_eq!(rows[0].reasoning_tokens, 50);
    assert_eq!(rows[0].cost_ticks, 125_000);
}

#[test]
fn aggregate_by_model_multiple_models() {
    let (_tmp, store) = open_store();
    store.sessions().create("s1", "Untrusted").unwrap();

    seed_transcript(
        &store,
        "s1",
        "/v1/responses",
        "grok-3",
        100,
        200,
        0,
        100_000,
    );
    seed_transcript(
        &store,
        "s1",
        "/v1/responses",
        "grok-4",
        200,
        400,
        100,
        500_000,
    );
    seed_transcript(
        &store,
        "s1",
        "/v1/responses",
        "grok-3-mini",
        50,
        100,
        0,
        10_000,
    );

    let rows = store
        .cost()
        .aggregate(CostGroupBy::Model, &CostFilter::default())
        .unwrap();
    assert_eq!(rows.len(), 3);

    // Rows sorted by cost descending.
    assert_eq!(rows[0].group, "grok-4");
    assert_eq!(rows[0].cost_ticks, 500_000);
    assert_eq!(rows[1].group, "grok-3");
    assert_eq!(rows[2].group, "grok-3-mini");
}

// ---------------------------------------------------------------------------
// Tests: Aggregation by endpoint
// ---------------------------------------------------------------------------

#[test]
fn aggregate_by_endpoint() {
    let (_tmp, store) = open_store();
    store.sessions().create("s1", "Untrusted").unwrap();

    seed_transcript(&store, "s1", "/v1/responses", "grok-3", 100, 200, 0, 50_000);
    seed_transcript(
        &store,
        "s1",
        "/v1/chat/completions",
        "grok-3",
        80,
        160,
        0,
        40_000,
    );
    seed_transcript(&store, "s1", "/v1/responses", "grok-3", 120, 240, 0, 60_000);

    let rows = store
        .cost()
        .aggregate(CostGroupBy::Endpoint, &CostFilter::default())
        .unwrap();
    assert_eq!(rows.len(), 2);

    // Find the responses endpoint row.
    let responses_row = rows.iter().find(|r| r.group == "/v1/responses").unwrap();
    assert_eq!(responses_row.requests, 2);
    assert_eq!(responses_row.cost_ticks, 110_000);

    let chat_row = rows
        .iter()
        .find(|r| r.group == "/v1/chat/completions")
        .unwrap();
    assert_eq!(chat_row.requests, 1);
    assert_eq!(chat_row.cost_ticks, 40_000);
}

// ---------------------------------------------------------------------------
// Tests: Aggregation by session
// ---------------------------------------------------------------------------

#[test]
fn aggregate_by_session() {
    let (_tmp, store) = open_store();
    store.sessions().create("s1", "Untrusted").unwrap();
    store.sessions().create("s2", "AdminTrusted").unwrap();

    seed_transcript(&store, "s1", "/v1/responses", "grok-3", 100, 200, 0, 50_000);
    seed_transcript(&store, "s1", "/v1/responses", "grok-3", 100, 200, 0, 50_000);
    seed_transcript(
        &store,
        "s2",
        "/v1/responses",
        "grok-4",
        200,
        400,
        100,
        200_000,
    );

    let rows = store
        .cost()
        .aggregate(CostGroupBy::Session, &CostFilter::default())
        .unwrap();
    assert_eq!(rows.len(), 2);

    // s2 has higher cost so should be first.
    assert_eq!(rows[0].group, "s2");
    assert_eq!(rows[0].cost_ticks, 200_000);
    assert_eq!(rows[0].requests, 1);

    assert_eq!(rows[1].group, "s1");
    assert_eq!(rows[1].cost_ticks, 100_000);
    assert_eq!(rows[1].requests, 2);
}

// ---------------------------------------------------------------------------
// Tests: Summary
// ---------------------------------------------------------------------------

#[test]
fn summary_across_all_transcripts() {
    let (_tmp, store) = open_store();
    store.sessions().create("s1", "Untrusted").unwrap();
    store.sessions().create("s2", "Untrusted").unwrap();

    seed_transcript(
        &store,
        "s1",
        "/v1/responses",
        "grok-3",
        100,
        200,
        10,
        50_000,
    );
    seed_transcript(
        &store,
        "s2",
        "/v1/responses",
        "grok-4",
        200,
        400,
        90,
        150_000,
    );

    let summary = store.cost().summary(&CostFilter::default()).unwrap();
    assert_eq!(summary.total_requests, 2);
    assert_eq!(summary.total_input_tokens, 300);
    assert_eq!(summary.total_output_tokens, 600);
    assert_eq!(summary.total_reasoning_tokens, 100);
    assert_eq!(summary.total_cost_ticks, 200_000);
    assert_eq!(summary.session_count, 2);
}

#[test]
fn summary_cost_usd_formatting() {
    let (_tmp, store) = open_store();
    store.sessions().create("s1", "Untrusted").unwrap();

    seed_transcript(
        &store,
        "s1",
        "/v1/responses",
        "grok-3",
        100,
        200,
        0,
        1_234_567,
    );

    let summary = store.cost().summary(&CostFilter::default()).unwrap();
    let usd = summary.total_cost_usd();
    assert_eq!(usd, "$1.234567");
}

#[test]
fn summary_avg_cost_per_session() {
    let (_tmp, store) = open_store();
    store.sessions().create("s1", "Untrusted").unwrap();
    store.sessions().create("s2", "Untrusted").unwrap();

    seed_transcript(
        &store,
        "s1",
        "/v1/responses",
        "grok-3",
        100,
        200,
        0,
        1_000_000,
    );
    seed_transcript(
        &store,
        "s2",
        "/v1/responses",
        "grok-3",
        100,
        200,
        0,
        3_000_000,
    );

    let summary = store.cost().summary(&CostFilter::default()).unwrap();
    assert_eq!(summary.avg_cost_per_session_ticks(), 2_000_000);
    assert_eq!(summary.avg_cost_per_session_usd(), "$2.000000");
}

// ---------------------------------------------------------------------------
// Tests: Filtering
// ---------------------------------------------------------------------------

#[test]
fn filter_by_session_id() {
    let (_tmp, store) = open_store();
    store.sessions().create("s1", "Untrusted").unwrap();
    store.sessions().create("s2", "Untrusted").unwrap();

    seed_transcript(&store, "s1", "/v1/responses", "grok-3", 100, 200, 0, 50_000);
    seed_transcript(
        &store,
        "s2",
        "/v1/responses",
        "grok-4",
        200,
        400,
        100,
        200_000,
    );

    let filter = CostFilter {
        session_id: Some("s1".to_owned()),
        ..Default::default()
    };
    let rows = store.cost().aggregate(CostGroupBy::Model, &filter).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].group, "grok-3");

    let summary = store.cost().summary(&filter).unwrap();
    assert_eq!(summary.total_requests, 1);
    assert_eq!(summary.session_count, 1);
}

// ---------------------------------------------------------------------------
// Tests: Empty data
// ---------------------------------------------------------------------------

#[test]
fn aggregate_empty_database_returns_empty() {
    let (_tmp, store) = open_store();
    let rows = store
        .cost()
        .aggregate(CostGroupBy::Model, &CostFilter::default())
        .unwrap();
    assert!(rows.is_empty());
}

#[test]
fn summary_empty_database() {
    let (_tmp, store) = open_store();
    let summary = store.cost().summary(&CostFilter::default()).unwrap();
    assert_eq!(summary.total_requests, 0);
    assert_eq!(summary.total_cost_ticks, 0);
    assert_eq!(summary.session_count, 0);
    assert_eq!(summary.avg_cost_per_session_ticks(), 0);
}

// ---------------------------------------------------------------------------
// Tests: Format helpers
// ---------------------------------------------------------------------------

#[test]
fn format_usd_basic() {
    assert_eq!(format_usd(0), "$0.000000");
    assert_eq!(format_usd(1_000_000), "$1.000000");
    assert_eq!(format_usd(1_500_000), "$1.500000");
    assert_eq!(format_usd(500), "$0.000500");
    assert_eq!(format_usd(1), "$0.000001");
}

#[test]
fn format_table_with_data() {
    let (_tmp, store) = open_store();
    store.sessions().create("s1", "Untrusted").unwrap();

    seed_transcript(&store, "s1", "/v1/responses", "grok-3", 100, 200, 0, 50_000);
    seed_transcript(
        &store,
        "s1",
        "/v1/responses",
        "grok-4",
        200,
        400,
        100,
        500_000,
    );

    let rows = store
        .cost()
        .aggregate(CostGroupBy::Model, &CostFilter::default())
        .unwrap();
    let summary = store.cost().summary(&CostFilter::default()).unwrap();

    let table = format_table(CostGroupBy::Model, &rows, &summary);
    assert!(table.contains("MODEL"), "table should have model header");
    assert!(table.contains("grok-4"));
    assert!(table.contains("grok-3"));
    assert!(table.contains("Total:"), "table should have summary line");
}

#[test]
fn format_table_empty_data() {
    let (_tmp, store) = open_store();
    let summary = store.cost().summary(&CostFilter::default()).unwrap();
    let table = format_table(CostGroupBy::Model, &[], &summary);
    assert!(table.contains("No usage data found"));
}

// ---------------------------------------------------------------------------
// Tests: CostRow accessors
// ---------------------------------------------------------------------------

#[test]
fn cost_row_cost_usd() {
    let (_tmp, store) = open_store();
    store.sessions().create("s1", "Untrusted").unwrap();

    seed_transcript(
        &store,
        "s1",
        "/v1/responses",
        "grok-3",
        100,
        200,
        0,
        2_500_000,
    );

    let rows = store
        .cost()
        .aggregate(CostGroupBy::Model, &CostFilter::default())
        .unwrap();
    assert_eq!(rows[0].cost_usd(), "$2.500000");
}

// ---------------------------------------------------------------------------
// Tests: CostGroupBy headers
// ---------------------------------------------------------------------------

#[test]
fn cost_group_by_headers() {
    assert_eq!(CostGroupBy::Model.header(), "model");
    assert_eq!(CostGroupBy::Day.header(), "date");
    assert_eq!(CostGroupBy::Session.header(), "session");
    assert_eq!(CostGroupBy::Endpoint.header(), "endpoint");
}

// ---------------------------------------------------------------------------
// Tests: Transcript without model in body
// ---------------------------------------------------------------------------

#[test]
fn aggregate_handles_missing_model_in_body() {
    let (_tmp, store) = open_store();
    store.sessions().create("s1", "Untrusted").unwrap();

    // Log a transcript with no request_body (model extraction falls back to "unknown").
    let tid = store
        .transcripts()
        .log_request("s1", "/v1/responses", "POST", None)
        .unwrap();
    let usage = TranscriptUsage {
        cost_in_usd_ticks: Some(10_000),
        input_tokens: Some(50),
        output_tokens: Some(100),
        reasoning_tokens: Some(0),
    };
    store
        .transcripts()
        .log_response(tid, 200, None, &usage, None)
        .unwrap();

    let rows = store
        .cost()
        .aggregate(CostGroupBy::Model, &CostFilter::default())
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].group, "unknown");
}
