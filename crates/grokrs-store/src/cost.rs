//! Cost aggregation queries: by model, by day, by session, by endpoint.
//!
//! `CostRepo` is a borrowed handle into the `Store`'s connection. It provides
//! SQL-level aggregation of cost and token data from the `transcripts` table,
//! with optional date range and session filtering.
//!
//! Model names are extracted best-effort from the `request_body` JSON column.
//! If the body does not parse or lacks a `"model"` field, the row is grouped
//! under `"unknown"`.

use rusqlite::{Connection, params_from_iter};
use serde::Serialize;
use std::fmt::Write as _;

use crate::StoreError;

/// A single row in a cost aggregation report.
///
/// The `group` field contains the grouping key: model name, date string,
/// session ID, or endpoint depending on the query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CostRow {
    /// The grouping key value.
    pub group: String,
    /// Number of transcript records in this group.
    pub requests: u64,
    /// Total input tokens.
    pub input_tokens: u64,
    /// Total output tokens.
    pub output_tokens: u64,
    /// Total reasoning tokens.
    pub reasoning_tokens: u64,
    /// Total cost in USD ticks (1_000_000 ticks = $1.00 USD).
    pub cost_ticks: i64,
}

impl CostRow {
    /// Format cost_ticks as a USD string with 6 decimal places.
    #[must_use]
    pub fn cost_usd(&self) -> String {
        format_usd(self.cost_ticks)
    }
}

/// The grouping dimension for a cost report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostGroupBy {
    /// Group by model name (extracted from request_body JSON).
    Model,
    /// Group by date (DATE(request_at)).
    Day,
    /// Group by session_id.
    Session,
    /// Group by API endpoint.
    Endpoint,
}

impl CostGroupBy {
    /// Human-readable header label for the group column.
    #[must_use]
    pub fn header(&self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Day => "date",
            Self::Session => "session",
            Self::Endpoint => "endpoint",
        }
    }
}

/// Filtering parameters for cost queries.
#[derive(Debug, Clone, Default)]
pub struct CostFilter {
    /// Only include transcripts at or after this date (YYYY-MM-DD or full timestamp).
    pub since: Option<String>,
    /// Only include transcripts before or at this date (YYYY-MM-DD or full timestamp).
    pub until: Option<String>,
    /// Only include transcripts for this session.
    pub session_id: Option<String>,
}

/// Aggregated cost summary across all matched rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CostSummary {
    /// Total number of requests.
    pub total_requests: u64,
    /// Total input tokens.
    pub total_input_tokens: u64,
    /// Total output tokens.
    pub total_output_tokens: u64,
    /// Total reasoning tokens.
    pub total_reasoning_tokens: u64,
    /// Total cost in USD ticks.
    pub total_cost_ticks: i64,
    /// Number of distinct sessions.
    pub session_count: u64,
}

impl CostSummary {
    /// Total cost as a USD string with 6 decimal places.
    #[must_use]
    pub fn total_cost_usd(&self) -> String {
        format_usd(self.total_cost_ticks)
    }

    /// Average cost per session in USD ticks. Returns 0 if session_count is 0.
    #[must_use]
    pub fn avg_cost_per_session_ticks(&self) -> i64 {
        if self.session_count == 0 {
            0
        } else {
            // RATIONALE: session_count is a small positive u64 from SQL COUNT;
            // values above i64::MAX are physically impossible (would require
            // >9.2 quintillion sessions).
            #[allow(clippy::cast_possible_wrap)]
            {
                self.total_cost_ticks / self.session_count as i64
            }
        }
    }

    /// Average cost per session as a USD string with 6 decimal places.
    #[must_use]
    pub fn avg_cost_per_session_usd(&self) -> String {
        format_usd(self.avg_cost_per_session_ticks())
    }
}

/// Format cost ticks as USD with 6 decimal places.
///
/// 1_000_000 ticks = $1.00 USD.
#[must_use]
pub fn format_usd(ticks: i64) -> String {
    // RATIONALE: precision loss is inherent to f64 display; sub-cent
    // accuracy beyond ~15 significant digits is irrelevant for USD formatting.
    #[allow(clippy::cast_precision_loss)]
    let dollars = ticks as f64 / 1_000_000.0;
    format!("${dollars:.6}")
}

/// Safely convert a non-negative `i64` (as returned by SQLite SUM/COUNT) to `u64`.
fn i64_to_u64(value: i64, column: &'static str) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::NegativeTokenCount { column, value })
}

/// Borrowed handle for cost aggregation queries.
pub struct CostRepo<'a> {
    conn: &'a Connection,
}

impl<'a> CostRepo<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Run a cost aggregation query grouped by the specified dimension.
    ///
    /// Returns rows sorted by cost descending (highest cost first).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NegativeTokenCount`] if any aggregated value is negative.
    /// Returns [`StoreError::Sql`] if the query fails.
    pub fn aggregate(
        &self,
        group_by: CostGroupBy,
        filter: &CostFilter,
    ) -> Result<Vec<CostRow>, StoreError> {
        // Build the GROUP BY expression.
        //
        // For Model grouping, we extract the model name from the JSON request_body
        // using SQLite's json_extract(). If request_body is NULL or doesn't contain
        // a "model" field, COALESCE falls back to 'unknown'.
        let group_expr = match group_by {
            CostGroupBy::Model => {
                "COALESCE(json_extract(request_body, '$.model'), 'unknown')".to_owned()
            }
            CostGroupBy::Day => "DATE(request_at)".to_owned(),
            CostGroupBy::Session => "session_id".to_owned(),
            CostGroupBy::Endpoint => "endpoint".to_owned(),
        };

        let (where_clause, bind_values) = build_where_clause(filter);

        let sql = format!(
            "SELECT \
                {group_expr} AS grp, \
                COUNT(*) AS requests, \
                COALESCE(SUM(COALESCE(input_tokens, 0)), 0) AS input_tokens, \
                COALESCE(SUM(COALESCE(output_tokens, 0)), 0) AS output_tokens, \
                COALESCE(SUM(COALESCE(reasoning_tokens, 0)), 0) AS reasoning_tokens, \
                COALESCE(SUM(COALESCE(cost_in_usd_ticks, 0)), 0) AS cost_ticks \
             FROM transcripts \
             {where_clause} \
             GROUP BY grp \
             ORDER BY cost_ticks DESC"
        );

        let mut stmt = self.conn.prepare(&sql).map_err(StoreError::Sql)?;
        let rows = stmt
            .query_map(params_from_iter(bind_values.iter()), |row| {
                Ok(RawCostRow {
                    group: row.get(0)?,
                    requests: row.get(1)?,
                    input_tokens: row.get(2)?,
                    output_tokens: row.get(3)?,
                    reasoning_tokens: row.get(4)?,
                    cost_ticks: row.get(5)?,
                })
            })
            .map_err(StoreError::Sql)?;

        let mut results = Vec::new();
        for row in rows {
            let raw = row.map_err(StoreError::Sql)?;
            results.push(raw.into_cost_row()?);
        }
        Ok(results)
    }

    /// Compute an overall summary across all transcripts matching the filter.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NegativeTokenCount`] if any aggregated value is negative.
    /// Returns [`StoreError::Sql`] if the query fails.
    pub fn summary(&self, filter: &CostFilter) -> Result<CostSummary, StoreError> {
        let (where_clause, bind_values) = build_where_clause(filter);

        let sql = format!(
            "SELECT \
                COUNT(*) AS total_requests, \
                COALESCE(SUM(COALESCE(input_tokens, 0)), 0) AS total_input, \
                COALESCE(SUM(COALESCE(output_tokens, 0)), 0) AS total_output, \
                COALESCE(SUM(COALESCE(reasoning_tokens, 0)), 0) AS total_reasoning, \
                COALESCE(SUM(COALESCE(cost_in_usd_ticks, 0)), 0) AS total_cost, \
                COUNT(DISTINCT session_id) AS session_count \
             FROM transcripts \
             {where_clause}"
        );

        let (total_requests, total_input, total_output, total_reasoning, total_cost, sessions) =
            self.conn
                .query_row(&sql, params_from_iter(bind_values.iter()), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                })
                .map_err(StoreError::Sql)?;

        Ok(CostSummary {
            total_requests: i64_to_u64(total_requests, "total_requests")?,
            total_input_tokens: i64_to_u64(total_input, "total_input_tokens")?,
            total_output_tokens: i64_to_u64(total_output, "total_output_tokens")?,
            total_reasoning_tokens: i64_to_u64(total_reasoning, "total_reasoning_tokens")?,
            total_cost_ticks: total_cost,
            session_count: i64_to_u64(sessions, "session_count")?,
        })
    }
}

/// Intermediate row before u64 validation.
struct RawCostRow {
    group: String,
    requests: i64,
    input_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
    cost_ticks: i64,
}

impl RawCostRow {
    fn into_cost_row(self) -> Result<CostRow, StoreError> {
        Ok(CostRow {
            group: self.group,
            requests: i64_to_u64(self.requests, "requests")?,
            input_tokens: i64_to_u64(self.input_tokens, "input_tokens")?,
            output_tokens: i64_to_u64(self.output_tokens, "output_tokens")?,
            reasoning_tokens: i64_to_u64(self.reasoning_tokens, "reasoning_tokens")?,
            cost_ticks: self.cost_ticks,
        })
    }
}

/// Build a WHERE clause and bind values from filter parameters.
///
/// Returns `("", vec![])` if no filters are set, or
/// `("WHERE condition1 AND condition2", vec![val1, val2])`.
fn build_where_clause(filter: &CostFilter) -> (String, Vec<String>) {
    let mut conditions = Vec::new();
    let mut values = Vec::new();

    if let Some(ref since) = filter.since {
        // If the user passes YYYY-MM-DD, we want >= that day.
        // If they pass a full timestamp, it works as-is.
        conditions.push(format!("request_at >= ?{}", values.len() + 1));
        values.push(normalise_date_filter(since, false));
    }

    if let Some(ref until) = filter.until {
        // For date-only input, include the entire day by using < next day.
        // For full timestamps, use <= as-is.
        conditions.push(format!("request_at <= ?{}", values.len() + 1));
        values.push(normalise_date_filter(until, true));
    }

    if let Some(ref session_id) = filter.session_id {
        conditions.push(format!("session_id = ?{}", values.len() + 1));
        values.push(session_id.clone());
    }

    if conditions.is_empty() {
        (String::new(), values)
    } else {
        let clause = format!("WHERE {}", conditions.join(" AND "));
        (clause, values)
    }
}

/// Normalise a date filter value.
///
/// If the value looks like a bare date (YYYY-MM-DD, 10 chars, no 'T'), append
/// a time component so it works correctly with RFC 3339 timestamps stored in
/// the database.
///
/// For `since` (is_upper=false): append `T00:00:00Z` to include from start of day.
/// For `until` (is_upper=true): append `T23:59:59Z` to include through end of day.
fn normalise_date_filter(value: &str, is_upper: bool) -> String {
    let trimmed = value.trim();
    // Check if it's a bare date: exactly 10 chars matching YYYY-MM-DD pattern.
    if trimmed.len() == 10 && !trimmed.contains('T') {
        if is_upper {
            format!("{trimmed}T23:59:59Z")
        } else {
            format!("{trimmed}T00:00:00Z")
        }
    } else {
        trimmed.to_owned()
    }
}

// ---------------------------------------------------------------------------
// Output formatters
// ---------------------------------------------------------------------------

/// Column widths for the cost table.
struct TableColumnWidths {
    group: usize,
    requests: usize,
    input: usize,
    output: usize,
    reasoning: usize,
    cost: usize,
}

impl TableColumnWidths {
    /// Compute column widths from the header label and data rows.
    fn compute(header: &str, rows: &[CostRow]) -> Self {
        let max_col = |f: fn(&CostRow) -> usize, minimum: usize| -> usize {
            rows.iter().map(f).max().unwrap_or(0).max(minimum)
        };
        Self {
            group: max_col(|r| r.group.len(), header.len().max(7)),
            requests: max_col(|r| format_u64(r.requests).len(), 8),
            input: max_col(|r| format_u64(r.input_tokens).len(), 12),
            output: max_col(|r| format_u64(r.output_tokens).len(), 13),
            reasoning: max_col(|r| format_u64(r.reasoning_tokens).len(), 16),
            cost: max_col(|r| r.cost_usd().len(), 10),
        }
    }

    fn total_width(&self) -> usize {
        self.group
            + 2
            + self.requests
            + 2
            + self.input
            + 2
            + self.output
            + 2
            + self.reasoning
            + 2
            + self.cost
    }

    /// Write a single data row into `out`.
    fn write_row(
        &self,
        out: &mut String,
        label: &str,
        req: &str,
        inp: &str,
        outp: &str,
        reas: &str,
        cost: &str,
    ) {
        write!(
            out,
            "{:<gw$}  {:>rw$}  {:>iw$}  {:>ow$}  {:>zw$}  {:>cw$}\n",
            label,
            req,
            inp,
            outp,
            reas,
            cost,
            gw = self.group,
            rw = self.requests,
            iw = self.input,
            ow = self.output,
            zw = self.reasoning,
            cw = self.cost,
        )
        .expect("String write is infallible");
    }
}

/// Format cost rows as a plain-text table.
///
/// Returns a multi-line string with fixed-width columns. The header row uses
/// the group_by dimension label. Rows are already sorted by the query.
#[must_use]
pub fn format_table(group_by: CostGroupBy, rows: &[CostRow], summary: &CostSummary) -> String {
    if rows.is_empty() {
        return "No usage data found.".to_owned();
    }

    let header = group_by.header();
    let widths = TableColumnWidths::compute(header, rows);
    let mut out = String::new();

    // Header line.
    widths.write_row(
        &mut out,
        &header.to_uppercase(),
        "REQUESTS",
        "INPUT_TOKENS",
        "OUTPUT_TOKENS",
        "REASONING_TOKENS",
        "COST_USD",
    );

    // Separator line.
    out.push_str(&"-".repeat(widths.total_width()));
    out.push('\n');

    // Data rows.
    for row in rows {
        widths.write_row(
            &mut out,
            &row.group,
            &format_u64(row.requests),
            &format_u64(row.input_tokens),
            &format_u64(row.output_tokens),
            &format_u64(row.reasoning_tokens),
            &row.cost_usd(),
        );
    }

    // Summary separator + summary line.
    out.push_str(&"=".repeat(widths.total_width()));
    out.push('\n');

    let total_tokens =
        summary.total_input_tokens + summary.total_output_tokens + summary.total_reasoning_tokens;
    write!(
        out,
        "Total: {} requests, {} tokens, {} | {} sessions, avg {}/session",
        format_u64(summary.total_requests),
        format_u64(total_tokens),
        summary.total_cost_usd(),
        format_u64(summary.session_count),
        summary.avg_cost_per_session_usd(),
    )
    .expect("String write is infallible");

    out
}

/// Format cost rows as JSON.
///
/// # Errors
///
/// Returns [`StoreError::Migration`] if JSON serialization fails.
pub fn format_json(rows: &[CostRow], summary: &CostSummary) -> Result<String, StoreError> {
    let output = serde_json::json!({
        "rows": rows.iter().map(|r| {
            serde_json::json!({
                "group": r.group,
                "requests": r.requests,
                "input_tokens": r.input_tokens,
                "output_tokens": r.output_tokens,
                "reasoning_tokens": r.reasoning_tokens,
                "cost_ticks": r.cost_ticks,
                "cost_usd": r.cost_usd(),
            })
        }).collect::<Vec<_>>(),
        "summary": {
            "total_requests": summary.total_requests,
            "total_input_tokens": summary.total_input_tokens,
            "total_output_tokens": summary.total_output_tokens,
            "total_reasoning_tokens": summary.total_reasoning_tokens,
            "total_cost_ticks": summary.total_cost_ticks,
            "total_cost_usd": summary.total_cost_usd(),
            "session_count": summary.session_count,
            "avg_cost_per_session_usd": summary.avg_cost_per_session_usd(),
        }
    });

    serde_json::to_string_pretty(&output)
        .map_err(|e| StoreError::Migration(format!("JSON serialization failed: {e}")))
}

/// Format cost rows as CSV.
#[must_use]
pub fn format_csv(group_by: CostGroupBy, rows: &[CostRow]) -> String {
    let mut out = String::new();

    // Header.
    write!(
        out,
        "{},requests,input_tokens,output_tokens,reasoning_tokens,cost_ticks,cost_usd\n",
        group_by.header()
    )
    .expect("String write is infallible");

    // Data rows.
    for row in rows {
        // CSV-escape the group value if it contains commas or quotes.
        let group = csv_escape(&row.group);
        write!(
            out,
            "{group},{},{},{},{},{},{}\n",
            row.requests,
            row.input_tokens,
            row.output_tokens,
            row.reasoning_tokens,
            row.cost_ticks,
            row.cost_usd(),
        )
        .expect("String write is infallible");
    }

    out
}

/// Escape a string value for CSV output.
///
/// If the value contains commas, double-quotes, or newlines, wrap it in
/// double-quotes and escape any internal double-quotes by doubling them.
fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        let escaped = value.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        value.to_owned()
    }
}

/// Format a u64 with thousands separators for display.
fn format_u64(value: u64) -> String {
    if value < 1_000 {
        return value.to_string();
    }

    let s = value.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    /// Helper: open an in-memory-like store in a temp directory, insert test data.
    fn setup_store_with_data() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();

        // Create sessions.
        store.sessions().create("s1", "Untrusted").unwrap();
        store.sessions().create("s2", "AdminTrusted").unwrap();

        // Insert transcripts with different models, endpoints, dates, and costs.
        let conn = store.conn_for_testing();

        // Session s1: two requests to chat completions with grok-3
        conn.execute(
            "INSERT INTO transcripts (session_id, request_at, response_at, endpoint, method, \
             request_body, status_code, cost_in_usd_ticks, input_tokens, output_tokens, \
             reasoning_tokens) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                "s1",
                "2025-01-15T10:00:00Z",
                "2025-01-15T10:00:01Z",
                "/v1/chat/completions",
                "POST",
                r#"{"model":"grok-3","messages":[]}"#,
                200,
                500_000_i64, // $0.50
                1000_i64,
                500_i64,
                100_i64,
            ],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO transcripts (session_id, request_at, response_at, endpoint, method, \
             request_body, status_code, cost_in_usd_ticks, input_tokens, output_tokens, \
             reasoning_tokens) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                "s1",
                "2025-01-15T11:00:00Z",
                "2025-01-15T11:00:02Z",
                "/v1/chat/completions",
                "POST",
                r#"{"model":"grok-3","messages":[]}"#,
                200,
                300_000_i64, // $0.30
                800_i64,
                400_i64,
                50_i64,
            ],
        )
        .unwrap();

        // Session s1: one request to responses endpoint with grok-3-mini
        conn.execute(
            "INSERT INTO transcripts (session_id, request_at, response_at, endpoint, method, \
             request_body, status_code, cost_in_usd_ticks, input_tokens, output_tokens, \
             reasoning_tokens) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                "s1",
                "2025-01-16T09:00:00Z",
                "2025-01-16T09:00:03Z",
                "/v1/responses",
                "POST",
                r#"{"model":"grok-3-mini","input":"hello"}"#,
                200,
                100_000_i64, // $0.10
                200_i64,
                150_i64,
                0_i64,
            ],
        )
        .unwrap();

        // Session s2: one request with no request_body (model = 'unknown')
        conn.execute(
            "INSERT INTO transcripts (session_id, request_at, response_at, endpoint, method, \
             request_body, status_code, cost_in_usd_ticks, input_tokens, output_tokens, \
             reasoning_tokens) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                "s2",
                "2025-01-16T14:00:00Z",
                "2025-01-16T14:00:01Z",
                "/v1/chat/completions",
                "POST",
                rusqlite::types::Null,
                200,
                50_000_i64, // $0.05
                100_i64,
                75_i64,
                25_i64,
            ],
        )
        .unwrap();

        (tmp, store)
    }

    // -----------------------------------------------------------------------
    // format_usd tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_usd_zero() {
        assert_eq!(format_usd(0), "$0.000000");
    }

    #[test]
    fn format_usd_one_dollar() {
        assert_eq!(format_usd(1_000_000), "$1.000000");
    }

    #[test]
    fn format_usd_sub_cent() {
        assert_eq!(format_usd(1), "$0.000001");
    }

    #[test]
    fn format_usd_fractional() {
        assert_eq!(format_usd(123_456), "$0.123456");
    }

    #[test]
    fn format_usd_large() {
        assert_eq!(format_usd(12_345_678), "$12.345678");
    }

    #[test]
    fn format_usd_negative() {
        assert_eq!(format_usd(-500_000), "$-0.500000");
    }

    // -----------------------------------------------------------------------
    // format_u64 tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_u64_small() {
        assert_eq!(format_u64(0), "0");
        assert_eq!(format_u64(999), "999");
    }

    #[test]
    fn format_u64_thousands() {
        assert_eq!(format_u64(1_000), "1,000");
        assert_eq!(format_u64(12_345), "12,345");
        assert_eq!(format_u64(1_234_567), "1,234,567");
    }

    // -----------------------------------------------------------------------
    // csv_escape tests
    // -----------------------------------------------------------------------

    #[test]
    fn csv_escape_plain() {
        assert_eq!(csv_escape("hello"), "hello");
    }

    #[test]
    fn csv_escape_with_comma() {
        assert_eq!(csv_escape("hello,world"), "\"hello,world\"");
    }

    #[test]
    fn csv_escape_with_quotes() {
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    // -----------------------------------------------------------------------
    // normalise_date_filter tests
    // -----------------------------------------------------------------------

    #[test]
    fn normalise_date_lower_bound() {
        assert_eq!(
            normalise_date_filter("2025-01-15", false),
            "2025-01-15T00:00:00Z"
        );
    }

    #[test]
    fn normalise_date_upper_bound() {
        assert_eq!(
            normalise_date_filter("2025-01-15", true),
            "2025-01-15T23:59:59Z"
        );
    }

    #[test]
    fn normalise_full_timestamp_passthrough() {
        let ts = "2025-01-15T12:30:00Z";
        assert_eq!(normalise_date_filter(ts, false), ts);
        assert_eq!(normalise_date_filter(ts, true), ts);
    }

    // -----------------------------------------------------------------------
    // Aggregate by model
    // -----------------------------------------------------------------------

    #[test]
    fn aggregate_by_model() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let rows = cost
            .aggregate(CostGroupBy::Model, &CostFilter::default())
            .unwrap();

        assert_eq!(rows.len(), 3);
        // Sorted by cost DESC: grok-3 ($0.80), grok-3-mini ($0.10), unknown ($0.05)
        assert_eq!(rows[0].group, "grok-3");
        assert_eq!(rows[0].cost_ticks, 800_000);
        assert_eq!(rows[0].requests, 2);
        assert_eq!(rows[0].input_tokens, 1800);
        assert_eq!(rows[0].output_tokens, 900);
        assert_eq!(rows[0].reasoning_tokens, 150);

        assert_eq!(rows[1].group, "grok-3-mini");
        assert_eq!(rows[1].cost_ticks, 100_000);
        assert_eq!(rows[1].requests, 1);

        assert_eq!(rows[2].group, "unknown");
        assert_eq!(rows[2].cost_ticks, 50_000);
        assert_eq!(rows[2].requests, 1);
    }

    // -----------------------------------------------------------------------
    // Aggregate by day
    // -----------------------------------------------------------------------

    #[test]
    fn aggregate_by_day() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let rows = cost
            .aggregate(CostGroupBy::Day, &CostFilter::default())
            .unwrap();

        assert_eq!(rows.len(), 2);
        // 2025-01-15: $0.80 (two grok-3 requests)
        // 2025-01-16: $0.15 (grok-3-mini + unknown)
        assert_eq!(rows[0].group, "2025-01-15");
        assert_eq!(rows[0].cost_ticks, 800_000);
        assert_eq!(rows[1].group, "2025-01-16");
        assert_eq!(rows[1].cost_ticks, 150_000);
    }

    // -----------------------------------------------------------------------
    // Aggregate by session
    // -----------------------------------------------------------------------

    #[test]
    fn aggregate_by_session() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let rows = cost
            .aggregate(CostGroupBy::Session, &CostFilter::default())
            .unwrap();

        assert_eq!(rows.len(), 2);
        // s1: $0.90 (3 requests), s2: $0.05 (1 request)
        assert_eq!(rows[0].group, "s1");
        assert_eq!(rows[0].cost_ticks, 900_000);
        assert_eq!(rows[0].requests, 3);
        assert_eq!(rows[1].group, "s2");
        assert_eq!(rows[1].cost_ticks, 50_000);
        assert_eq!(rows[1].requests, 1);
    }

    // -----------------------------------------------------------------------
    // Aggregate by endpoint
    // -----------------------------------------------------------------------

    #[test]
    fn aggregate_by_endpoint() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let rows = cost
            .aggregate(CostGroupBy::Endpoint, &CostFilter::default())
            .unwrap();

        assert_eq!(rows.len(), 2);
        // /v1/chat/completions: $0.85, /v1/responses: $0.10
        assert_eq!(rows[0].group, "/v1/chat/completions");
        assert_eq!(rows[0].cost_ticks, 850_000);
        assert_eq!(rows[0].requests, 3);
        assert_eq!(rows[1].group, "/v1/responses");
        assert_eq!(rows[1].cost_ticks, 100_000);
        assert_eq!(rows[1].requests, 1);
    }

    // -----------------------------------------------------------------------
    // Summary
    // -----------------------------------------------------------------------

    #[test]
    fn summary_all() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let summary = cost.summary(&CostFilter::default()).unwrap();

        assert_eq!(summary.total_requests, 4);
        assert_eq!(summary.total_input_tokens, 2100);
        assert_eq!(summary.total_output_tokens, 1125);
        assert_eq!(summary.total_reasoning_tokens, 175);
        assert_eq!(summary.total_cost_ticks, 950_000);
        assert_eq!(summary.session_count, 2);
        assert_eq!(summary.avg_cost_per_session_ticks(), 475_000);
    }

    // -----------------------------------------------------------------------
    // Filtering: --since / --until
    // -----------------------------------------------------------------------

    #[test]
    fn filter_since_date() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let filter = CostFilter {
            since: Some("2025-01-16".to_owned()),
            ..Default::default()
        };
        let rows = cost.aggregate(CostGroupBy::Day, &filter).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].group, "2025-01-16");
    }

    #[test]
    fn filter_until_date() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let filter = CostFilter {
            until: Some("2025-01-15".to_owned()),
            ..Default::default()
        };
        let rows = cost.aggregate(CostGroupBy::Day, &filter).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].group, "2025-01-15");
    }

    #[test]
    fn filter_since_and_until() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let filter = CostFilter {
            since: Some("2025-01-15".to_owned()),
            until: Some("2025-01-15".to_owned()),
            ..Default::default()
        };
        let rows = cost.aggregate(CostGroupBy::Model, &filter).unwrap();
        // Only the two grok-3 requests on 2025-01-15
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].group, "grok-3");
        assert_eq!(rows[0].requests, 2);
    }

    // -----------------------------------------------------------------------
    // Filtering: --session
    // -----------------------------------------------------------------------

    #[test]
    fn filter_session() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let filter = CostFilter {
            session_id: Some("s2".to_owned()),
            ..Default::default()
        };
        let rows = cost.aggregate(CostGroupBy::Model, &filter).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].group, "unknown");
        assert_eq!(rows[0].requests, 1);
    }

    #[test]
    fn filter_session_summary() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let filter = CostFilter {
            session_id: Some("s1".to_owned()),
            ..Default::default()
        };
        let summary = cost.summary(&filter).unwrap();
        assert_eq!(summary.total_requests, 3);
        assert_eq!(summary.total_cost_ticks, 900_000);
        assert_eq!(summary.session_count, 1);
    }

    // -----------------------------------------------------------------------
    // Empty store
    // -----------------------------------------------------------------------

    #[test]
    fn empty_store_aggregate() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let cost = store.cost();
        let rows = cost
            .aggregate(CostGroupBy::Model, &CostFilter::default())
            .unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn empty_store_summary() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let cost = store.cost();
        let summary = cost.summary(&CostFilter::default()).unwrap();
        assert_eq!(summary.total_requests, 0);
        assert_eq!(summary.total_cost_ticks, 0);
        assert_eq!(summary.session_count, 0);
    }

    // -----------------------------------------------------------------------
    // Output format tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_table_empty() {
        let summary = CostSummary {
            total_requests: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_reasoning_tokens: 0,
            total_cost_ticks: 0,
            session_count: 0,
        };
        let output = format_table(CostGroupBy::Model, &[], &summary);
        assert_eq!(output, "No usage data found.");
    }

    #[test]
    fn format_table_with_data() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let rows = cost
            .aggregate(CostGroupBy::Model, &CostFilter::default())
            .unwrap();
        let summary = cost.summary(&CostFilter::default()).unwrap();
        let output = format_table(CostGroupBy::Model, &rows, &summary);

        // Check key elements are present.
        assert!(output.contains("MODEL"));
        assert!(output.contains("REQUESTS"));
        assert!(output.contains("COST_USD"));
        assert!(output.contains("grok-3"));
        assert!(output.contains("grok-3-mini"));
        assert!(output.contains("unknown"));
        assert!(output.contains("Total:"));
        assert!(output.contains("$0.950000"));
    }

    #[test]
    fn format_json_output() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let rows = cost
            .aggregate(CostGroupBy::Model, &CostFilter::default())
            .unwrap();
        let summary = cost.summary(&CostFilter::default()).unwrap();
        let json_str = format_json(&rows, &summary).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed["rows"].is_array());
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 3);
        assert_eq!(parsed["summary"]["total_requests"], 4);
        assert_eq!(parsed["summary"]["total_cost_usd"], "$0.950000");
    }

    #[test]
    fn format_csv_output() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let rows = cost
            .aggregate(CostGroupBy::Model, &CostFilter::default())
            .unwrap();
        let csv_str = format_csv(CostGroupBy::Model, &rows);

        let lines: Vec<&str> = csv_str.lines().collect();
        // Header + 3 data rows
        assert_eq!(lines.len(), 4);
        assert_eq!(
            lines[0],
            "model,requests,input_tokens,output_tokens,reasoning_tokens,cost_ticks,cost_usd"
        );
        assert!(lines[1].starts_with("grok-3,"));
    }

    // -----------------------------------------------------------------------
    // CostRow::cost_usd
    // -----------------------------------------------------------------------

    #[test]
    fn cost_row_cost_usd() {
        let row = CostRow {
            group: "test".to_owned(),
            requests: 1,
            input_tokens: 100,
            output_tokens: 50,
            reasoning_tokens: 0,
            cost_ticks: 123_456,
        };
        assert_eq!(row.cost_usd(), "$0.123456");
    }

    // -----------------------------------------------------------------------
    // CostSummary methods
    // -----------------------------------------------------------------------

    #[test]
    fn summary_avg_cost_zero_sessions() {
        let summary = CostSummary {
            total_requests: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_reasoning_tokens: 0,
            total_cost_ticks: 0,
            session_count: 0,
        };
        assert_eq!(summary.avg_cost_per_session_ticks(), 0);
        assert_eq!(summary.avg_cost_per_session_usd(), "$0.000000");
    }

    #[test]
    fn summary_avg_cost_with_sessions() {
        let summary = CostSummary {
            total_requests: 10,
            total_input_tokens: 5000,
            total_output_tokens: 2500,
            total_reasoning_tokens: 500,
            total_cost_ticks: 2_000_000,
            session_count: 4,
        };
        assert_eq!(summary.avg_cost_per_session_ticks(), 500_000);
        assert_eq!(summary.avg_cost_per_session_usd(), "$0.500000");
    }

    // -----------------------------------------------------------------------
    // Combined filters
    // -----------------------------------------------------------------------

    #[test]
    fn filter_session_and_date_range() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let filter = CostFilter {
            since: Some("2025-01-16".to_owned()),
            session_id: Some("s1".to_owned()),
            ..Default::default()
        };
        let rows = cost.aggregate(CostGroupBy::Model, &filter).unwrap();
        // Only s1's request on 2025-01-16 (grok-3-mini)
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].group, "grok-3-mini");
        assert_eq!(rows[0].requests, 1);
    }

    #[test]
    fn filter_no_match() {
        let (_tmp, store) = setup_store_with_data();
        let cost = store.cost();
        let filter = CostFilter {
            session_id: Some("nonexistent".to_owned()),
            ..Default::default()
        };
        let rows = cost.aggregate(CostGroupBy::Model, &filter).unwrap();
        assert!(rows.is_empty());

        let summary = cost.summary(&filter).unwrap();
        assert_eq!(summary.total_requests, 0);
    }
}
