use chrono::{DateTime, NaiveDate, Utc};
use once_cell::sync::Lazy;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::mpsc::{self, SyncSender};
use std::thread;
use std::time::Duration;

use crate::error::AppResult;
use crate::paths::db_path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLog {
    pub request_id: String,
    pub account_id: Option<String>,
    pub endpoint: String,
    pub requested_model: Option<String>,
    pub resolved_model: Option<String>,
    pub status_code: u16,
    pub latency_ms: u64,
    pub first_token_ms: Option<u64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_tokens: u64,
    pub estimated_cost_usd: f64,
    pub error_summary: Option<String>,
    pub client_source: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub total_requests: u64,
    pub success_requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_tokens: u64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapDay {
    pub date: String,
    pub requests: u64,
    pub tokens: u64,
    pub cost_usd: f64,
}

/// Bound the log queue so a stuck writer cannot grow RAM without limit.
const LOG_QUEUE_CAP: usize = 2048;
/// Keep at most this many rows (oldest deleted first).
const MAX_LOG_ROWS: i64 = 50_000;
/// Drop rows older than this many days.
const LOG_RETENTION_DAYS: i64 = 30;
/// Prune / checkpoint every N inserts on the writer thread.
const PRUNE_EVERY_N: u32 = 64;

static LOG_TX: Lazy<SyncSender<RequestLog>> = Lazy::new(|| {
    let (tx, rx) = mpsc::sync_channel::<RequestLog>(LOG_QUEUE_CAP);
    thread::Builder::new()
        .name("usage-log-writer".into())
        .spawn(move || {
            let Ok(store) = UsageStore::open_default() else {
                tracing::error!("usage log writer: failed to open database");
                // Drain so senders don't block forever if open failed.
                while rx.recv().is_ok() {}
                return;
            };
            let _ = store.prune(LOG_RETENTION_DAYS, MAX_LOG_ROWS);
            let _ = store.checkpoint();
            let mut since_prune = 0u32;
            while let Ok(log) = rx.recv() {
                if let Err(err) = store.insert(&log) {
                    tracing::warn!("usage log insert failed: {err}");
                    continue;
                }
                since_prune += 1;
                if since_prune >= PRUNE_EVERY_N {
                    since_prune = 0;
                    if let Err(err) = store.prune(LOG_RETENTION_DAYS, MAX_LOG_ROWS) {
                        tracing::warn!("usage prune failed: {err}");
                    }
                    if let Err(err) = store.checkpoint() {
                        tracing::warn!("usage wal checkpoint failed: {err}");
                    }
                }
            }
        })
        .expect("spawn usage log writer");
    tx
});

/// Non-blocking enqueue; drops when the queue is full (prefer proxy latency over perfect logs).
pub fn enqueue_request_log(log: RequestLog) {
    match LOG_TX.try_send(log) {
        Ok(()) => {}
        Err(mpsc::TrySendError::Full(_)) => {
            tracing::warn!("usage log queue full; dropping log entry");
        }
        Err(mpsc::TrySendError::Disconnected(_)) => {
            tracing::warn!("usage log writer disconnected");
        }
    }
}

/// Ensure the background writer thread is running (idempotent via Lazy).
pub fn init_log_writer() {
    Lazy::force(&LOG_TX);
}

pub struct UsageStore {
    conn: Connection,
}

impl UsageStore {
    pub fn open_default() -> AppResult<Self> {
        Self::open(db_path()?)
    }

    pub fn open(path: impl AsRef<Path>) -> AppResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        // Gateway writes while UI reads; without WAL/busy timeout the UI stays on
        // "Loading…" forever with database-is-locked errors. Fresh installs race the
        // writer thread vs first UI query — keep timeout generous.
        conn.busy_timeout(Duration::from_secs(15))?;
        // journal_mode must be set outside a multi-statement batch on some platforms.
        let _ = conn.query_row("PRAGMA journal_mode=WAL;", [], |row| row.get::<_, String>(0));
        let _ = conn.execute_batch(
            r#"
            PRAGMA synchronous=NORMAL;
            PRAGMA temp_store=MEMORY;
            PRAGMA cache_size=-8000;
            PRAGMA busy_timeout=15000;
            "#,
        );
        let store = Self { conn };
        store.init()?;
        Ok(store)
    }

    fn init(&self) -> AppResult<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS request_logs (
              request_id TEXT PRIMARY KEY,
              account_id TEXT,
              endpoint TEXT NOT NULL,
              requested_model TEXT,
              resolved_model TEXT,
              status_code INTEGER NOT NULL,
              latency_ms INTEGER NOT NULL,
              first_token_ms INTEGER,
              input_tokens INTEGER NOT NULL DEFAULT 0,
              output_tokens INTEGER NOT NULL DEFAULT 0,
              cache_tokens INTEGER NOT NULL DEFAULT 0,
              estimated_cost_usd REAL NOT NULL DEFAULT 0,
              error_summary TEXT,
              client_source TEXT NOT NULL,
              created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_request_logs_created_at ON request_logs(created_at);
            CREATE INDEX IF NOT EXISTS idx_request_logs_account ON request_logs(account_id);
            "#,
        )?;
        Ok(())
    }

    pub fn insert(&self, log: &RequestLog) -> AppResult<()> {
        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO request_logs (
              request_id, account_id, endpoint, requested_model, resolved_model,
              status_code, latency_ms, first_token_ms, input_tokens, output_tokens,
              cache_tokens, estimated_cost_usd, error_summary, client_source, created_at
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
            "#,
            params![
                log.request_id,
                log.account_id,
                log.endpoint,
                log.requested_model,
                log.resolved_model,
                log.status_code as i64,
                log.latency_ms as i64,
                log.first_token_ms.map(|v| v as i64),
                log.input_tokens as i64,
                log.output_tokens as i64,
                log.cache_tokens as i64,
                log.estimated_cost_usd,
                log.error_summary,
                log.client_source,
                log.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Drop old rows and cap total size to bound disk + mmap growth over long runs.
    pub fn prune(&self, retention_days: i64, max_rows: i64) -> AppResult<()> {
        let cutoff = (Utc::now() - chrono::Duration::days(retention_days.max(1))).to_rfc3339();
        self.conn.execute(
            "DELETE FROM request_logs WHERE created_at < ?1",
            params![cutoff],
        )?;
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM request_logs", [], |row| row.get(0))?;
        if count > max_rows {
            let excess = count - max_rows;
            self.conn.execute(
                r#"
                DELETE FROM request_logs WHERE request_id IN (
                  SELECT request_id FROM request_logs
                  ORDER BY created_at ASC
                  LIMIT ?1
                )
                "#,
                params![excess],
            )?;
        }
        Ok(())
    }

    /// Truncate WAL so long-running processes do not retain multi-GB `-wal` files.
    pub fn checkpoint(&self) -> AppResult<()> {
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        Ok(())
    }

    pub fn recent(&self, limit: usize, offset: usize) -> AppResult<Vec<RequestLog>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT request_id, account_id, endpoint, requested_model, resolved_model,
                   status_code, latency_ms, first_token_ms, input_tokens, output_tokens,
                   cache_tokens, estimated_cost_usd, error_summary, client_source, created_at
            FROM request_logs
            ORDER BY created_at DESC
            LIMIT ?1 OFFSET ?2
            "#,
        )?;
        let rows = stmt.query_map(params![limit as i64, offset as i64], |row| {
            let created_at: String = row.get(14)?;
            Ok(RequestLog {
                request_id: row.get(0)?,
                account_id: row.get(1)?,
                endpoint: row.get(2)?,
                requested_model: row.get(3)?,
                resolved_model: row.get(4)?,
                status_code: row.get::<_, i64>(5)? as u16,
                latency_ms: row.get::<_, i64>(6)? as u64,
                first_token_ms: row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
                input_tokens: row.get::<_, i64>(8)? as u64,
                output_tokens: row.get::<_, i64>(9)? as u64,
                cache_tokens: row.get::<_, i64>(10)? as u64,
                estimated_cost_usd: row.get(11)?,
                error_summary: row.get(12)?,
                client_source: row.get(13)?,
                created_at: DateTime::parse_from_rfc3339(&created_at)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn today_summary(&self) -> AppResult<UsageSummary> {
        let today = Utc::now().date_naive();
        self.summary_for_date(today)
    }

    pub fn summary_for_date(&self, date: NaiveDate) -> AppResult<UsageSummary> {
        let start = format!("{date}T00:00:00Z");
        let end = format!("{date}T23:59:59Z");
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
              COUNT(*),
              COALESCE(SUM(CASE WHEN status_code >= 200 AND status_code < 400 THEN 1 ELSE 0 END), 0),
              COALESCE(SUM(input_tokens), 0),
              COALESCE(SUM(output_tokens), 0),
              COALESCE(SUM(cache_tokens), 0),
              COALESCE(SUM(estimated_cost_usd), 0.0)
            FROM request_logs
            WHERE created_at >= ?1 AND created_at <= ?2
            "#,
        )?;
        let summary = stmt.query_row(params![start, end], |row| {
            Ok(UsageSummary {
                total_requests: row.get::<_, i64>(0).unwrap_or(0) as u64,
                success_requests: row
                    .get::<_, Option<i64>>(1)?
                    .unwrap_or(0) as u64,
                input_tokens: row.get::<_, Option<i64>>(2)?.unwrap_or(0) as u64,
                output_tokens: row.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                cache_tokens: row.get::<_, Option<i64>>(4)?.unwrap_or(0) as u64,
                estimated_cost_usd: row.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
            })
        })?;
        Ok(summary)
    }

    pub fn heatmap(&self, days: i64) -> AppResult<Vec<HeatmapDay>> {
        // Align calendar days to local timezone so every month day is covered for the user.
        let today = chrono::Local::now().date_naive();
        let days = days.max(1);
        let start_date = today
            .checked_sub_signed(chrono::Duration::days(days.saturating_sub(1)))
            .unwrap_or(today);
        // Query a bit earlier in UTC so local midnights near the range start are not dropped.
        let start = (start_date - chrono::Duration::days(1))
            .format("%Y-%m-%dT00:00:00")
            .to_string();

        let mut by_day = std::collections::HashMap::<String, (u64, u64, f64)>::new();
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
              created_at,
              input_tokens,
              output_tokens,
              cache_tokens,
              estimated_cost_usd
            FROM request_logs
            WHERE created_at >= ?1
            "#,
        )?;
        let rows = stmt.query_map(params![start], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as u64,
                row.get::<_, i64>(3)? as u64,
                row.get::<_, f64>(4)?,
            ))
        })?;
        for row in rows {
            let (created_at, input_tokens, output_tokens, cache_tokens, cost) = row?;
            // Map timestamp → local calendar day (YYYY-MM-DD).
            let day = DateTime::parse_from_rfc3339(&created_at)
                .map(|d| d.with_timezone(&chrono::Local).date_naive().to_string())
                .unwrap_or_else(|_| created_at.chars().take(10).collect());
            let tokens = total_tokens(input_tokens, output_tokens, cache_tokens);
            let entry = by_day.entry(day).or_insert((0, 0, 0.0));
            entry.0 += 1;
            entry.1 += tokens;
            entry.2 += cost;
        }

        // Emit every calendar day in range (including zero-activity days / months).
        let mut out = Vec::with_capacity(days as usize);
        for offset in (0..days).rev() {
            let date = today
                .checked_sub_signed(chrono::Duration::days(offset))
                .unwrap_or(today);
            let key = date.to_string();
            let (requests, tokens, cost_usd) = by_day.get(&key).copied().unwrap_or((0, 0, 0.0));
            out.push(HeatmapDay {
                date: key,
                requests,
                tokens,
                cost_usd,
            });
        }
        Ok(out)
    }

    pub fn clear(&self) -> AppResult<()> {
        self.conn.execute("DELETE FROM request_logs", [])?;
        self.checkpoint()?;
        Ok(())
    }
}

pub fn empty_summary() -> UsageSummary {
    UsageSummary {
        total_requests: 0,
        success_requests: 0,
        input_tokens: 0,
        output_tokens: 0,
        cache_tokens: 0,
        estimated_cost_usd: 0.0,
    }
}

pub fn estimate_cost(input_tokens: u64, output_tokens: u64, cache_tokens: u64) -> f64 {
    // rough placeholder pricing for UI until official rates are configured.
    // `input_tokens` is total prompt (includes cache reads). Bill uncached at full
    // rate and cached subset at the discounted rate — never charge full+cache.
    let cache = cache_tokens.min(input_tokens);
    let uncached = input_tokens.saturating_sub(cache);
    let input = uncached as f64 / 1_000_000.0 * 3.0;
    let cached = cache as f64 / 1_000_000.0 * 0.75;
    let output = output_tokens as f64 / 1_000_000.0 * 15.0;
    input + cached + output
}

/// Total tokens for a single request for display/aggregation.
/// Prompt `input` already includes cache hits — do not add `cache` again.
pub fn total_tokens(input_tokens: u64, output_tokens: u64, _cache_tokens: u64) -> u64 {
    input_tokens.saturating_add(output_tokens)
}


#[cfg(test)]
mod usage_store_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("grok-go-usage-test-{nanos}.db"))
    }

    #[test]
    fn empty_db_summary_and_heatmap_ok() {
        let path = temp_db();
        let _ = std::fs::remove_file(&path);
        let store = UsageStore::open(&path).expect("open empty db");
        let summary = store.today_summary().expect("today summary");
        assert_eq!(summary.total_requests, 0);
        assert_eq!(summary.success_requests, 0);
        assert_eq!(summary.input_tokens, 0);
        let heat = store.heatmap(14).expect("heatmap");
        assert_eq!(heat.len(), 14);
        assert!(heat.iter().all(|d| d.requests == 0));
        let recent = store.recent(10, 0).expect("recent");
        assert!(recent.is_empty());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn insert_then_summary() {
        let path = temp_db();
        let _ = std::fs::remove_file(&path);
        let store = UsageStore::open(&path).expect("open");
        store
            .insert(&RequestLog {
                request_id: "r1".into(),
                account_id: Some("a".into()),
                endpoint: "/v1/responses".into(),
                requested_model: Some("gpt-5.5".into()),
                resolved_model: Some("grok-4.5".into()),
                status_code: 200,
                latency_ms: 12,
                first_token_ms: Some(5),
                input_tokens: 10,
                output_tokens: 20,
                cache_tokens: 0,
                estimated_cost_usd: 0.01,
                error_summary: None,
                client_source: "test".into(),
                created_at: Utc::now(),
            })
            .expect("insert");
        let summary = store.today_summary().expect("summary");
        assert_eq!(summary.total_requests, 1);
        assert_eq!(summary.success_requests, 1);
        assert_eq!(summary.input_tokens, 10);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }
}
