use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_WINDOW_DAYS: u32 = 30;
const DEFAULT_WINDOW_HOURS: u32 = 24;
const MAX_WINDOW_DAYS: u32 = 365;
const MAX_WINDOW_HOURS: u32 = MAX_WINDOW_DAYS * 24;
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

static DATABASE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayUsageRecordResult {
    pub inserted: bool,
    pub event_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayUsageSummary {
    pub database_path: String,
    pub window_days: u32,
    pub window_hours: u32,
    pub start_date: String,
    pub end_date: String,
    pub generated_at_unix: i64,
    pub totals: GatewayUsageTotals,
    pub daily: Vec<GatewayUsageDaily>,
    pub by_provider: Vec<GatewayUsageBreakdown>,
    pub by_model: Vec<GatewayUsageBreakdown>,
    pub by_session: Vec<GatewayUsageSessionBreakdown>,
    pub by_project: Vec<GatewayUsageProjectBreakdown>,
    pub requests: Vec<GatewayUsageRequestEvent>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayUsageTotals {
    pub request_count: i64,
    pub success_count: i64,
    pub error_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub last_received_at_unix: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayUsageDaily {
    pub day: String,
    pub request_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayUsageBreakdown {
    pub label: String,
    pub provider: String,
    pub provider_name: String,
    pub model: String,
    pub request_count: i64,
    pub success_count: i64,
    pub error_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayUsageSessionBreakdown {
    pub session_id: String,
    pub label: String,
    pub project_path: String,
    pub project_label: String,
    pub request_count: i64,
    pub success_count: i64,
    pub error_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub first_received_at_unix: Option<i64>,
    pub last_received_at_unix: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayUsageProjectBreakdown {
    pub project_path: String,
    pub label: String,
    pub session_count: i64,
    pub request_count: i64,
    pub success_count: i64,
    pub error_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub first_received_at_unix: Option<i64>,
    pub last_received_at_unix: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayUsageRequestEvent {
    pub event_id: String,
    pub request_id: String,
    pub emitted_at: String,
    pub received_at_unix: i64,
    pub client_session_id: String,
    pub client_session_label: String,
    pub client_project_path: String,
    pub client_project_label: String,
    pub route: String,
    pub provider: String,
    pub provider_name: String,
    pub model: String,
    pub status: String,
    pub status_code: Option<i64>,
    pub latency_ms: Option<i64>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone)]
struct GatewayUsageEvent {
    event_id: String,
    emitted_at: String,
    received_at_unix: i64,
    request_id: String,
    route_method: String,
    route_url: String,
    source_provider: String,
    source_adapter_key: String,
    target_provider: String,
    target_provider_name: String,
    target_model: String,
    outcome_status: String,
    outcome_status_code: Option<i64>,
    error_message: String,
    latency_ms: Option<i64>,
    fallback_used: bool,
    fallback_attempts: i64,
    identity_user_id: String,
    identity_tenant_id: String,
    client_agent_id: String,
    client_session_id: String,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    total_tokens: i64,
    cache_duration_seconds: i64,
    input_cost: f64,
    output_cost: f64,
    cache_read_cost: f64,
    cache_write_cost: f64,
    total_cost: f64,
    currency: String,
}

#[derive(Debug, Clone)]
struct GatewayUsageDateRange {
    window_days: u32,
    window_hours: u32,
    start_date: String,
    end_date: String,
    start_unix: i64,
    end_unix_exclusive: i64,
}

#[derive(Debug, Clone, Default)]
struct CodexSessionMetadata {
    id: String,
    title: String,
    project_path: String,
    started_at_unix: Option<i64>,
    last_seen_at_unix: Option<i64>,
    is_subagent: bool,
}

#[derive(Debug, Clone, Default)]
struct CodexSessionIndexEntry {
    title: String,
    updated_at_unix: Option<i64>,
}

#[derive(Debug, Clone, Default)]
struct CodexSessionCatalog {
    by_id: HashMap<String, CodexSessionMetadata>,
    timeline: Vec<CodexSessionMetadata>,
}

pub async fn record_usage_report(payload: Value) -> Result<GatewayUsageRecordResult, String> {
    tokio::task::spawn_blocking(move || {
        let _guard = database_lock()
            .lock()
            .map_err(|_| "Gateway usage database lock is poisoned".to_string())?;
        let connection = open_database()?;
        insert_usage_event(&connection, payload)
    })
    .await
    .map_err(|err| err.to_string())?
}

pub async fn load_usage_summary(
    days: Option<u32>,
    start_date: Option<String>,
    end_date: Option<String>,
    hours: Option<u32>,
    codex_home: Option<PathBuf>,
) -> Result<GatewayUsageSummary, String> {
    let range = normalize_date_range(days, start_date, end_date, hours)?;
    tokio::task::spawn_blocking(move || {
        let _guard = database_lock()
            .lock()
            .map_err(|_| "Gateway usage database lock is poisoned".to_string())?;
        let database_path = database_path();
        let connection = open_database()?;
        load_summary_from_connection(
            &connection,
            range,
            database_path.to_string_lossy().to_string(),
            codex_home,
        )
    })
    .await
    .map_err(|err| err.to_string())?
}

fn database_lock() -> &'static Mutex<()> {
    DATABASE_LOCK.get_or_init(|| Mutex::new(()))
}

fn open_database() -> Result<Connection, String> {
    let path = database_path();
    let database_exists = path.is_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let connection = Connection::open(&path).map_err(|err| err.to_string())?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|err| err.to_string())?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|err| err.to_string())?;
    if !database_exists {
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|err| err.to_string())?;
        connection
            .pragma_update(None, "synchronous", "NORMAL")
            .map_err(|err| err.to_string())?;
    }
    init_database(&connection)?;
    Ok(connection)
}

fn init_database(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS gateway_usage_events (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              event_id TEXT NOT NULL UNIQUE,
              emitted_at TEXT NOT NULL DEFAULT '',
              received_at_unix INTEGER NOT NULL,
              request_id TEXT NOT NULL DEFAULT '',
              route_method TEXT NOT NULL DEFAULT '',
              route_url TEXT NOT NULL DEFAULT '',
              source_provider TEXT NOT NULL DEFAULT '',
              source_adapter_key TEXT NOT NULL DEFAULT '',
              target_provider TEXT NOT NULL DEFAULT '',
              target_provider_name TEXT NOT NULL DEFAULT '',
              target_model TEXT NOT NULL DEFAULT '',
              outcome_status TEXT NOT NULL DEFAULT '',
              outcome_status_code INTEGER,
              error_message TEXT NOT NULL DEFAULT '',
              latency_ms INTEGER,
              fallback_used INTEGER NOT NULL DEFAULT 0,
              fallback_attempts INTEGER NOT NULL DEFAULT 0,
              identity_user_id TEXT NOT NULL DEFAULT '',
              identity_tenant_id TEXT NOT NULL DEFAULT '',
              client_agent_id TEXT NOT NULL DEFAULT '',
              client_session_id TEXT NOT NULL DEFAULT '',
              input_tokens INTEGER NOT NULL DEFAULT 0,
              output_tokens INTEGER NOT NULL DEFAULT 0,
              cache_read_tokens INTEGER NOT NULL DEFAULT 0,
              cache_write_tokens INTEGER NOT NULL DEFAULT 0,
              total_tokens INTEGER NOT NULL DEFAULT 0,
              cache_duration_seconds INTEGER NOT NULL DEFAULT 0,
              input_cost REAL NOT NULL DEFAULT 0,
              output_cost REAL NOT NULL DEFAULT 0,
              cache_read_cost REAL NOT NULL DEFAULT 0,
              cache_write_cost REAL NOT NULL DEFAULT 0,
              total_cost REAL NOT NULL DEFAULT 0,
              currency TEXT NOT NULL DEFAULT 'USD'
            );
            CREATE INDEX IF NOT EXISTS idx_gateway_usage_received_at
              ON gateway_usage_events(received_at_unix);
            CREATE INDEX IF NOT EXISTS idx_gateway_usage_target
              ON gateway_usage_events(target_provider, target_provider_name, target_model);
            CREATE INDEX IF NOT EXISTS idx_gateway_usage_outcome
              ON gateway_usage_events(outcome_status, outcome_status_code);
            CREATE INDEX IF NOT EXISTS idx_gateway_usage_request_id
              ON gateway_usage_events(request_id);
            CREATE INDEX IF NOT EXISTS idx_gateway_usage_client_session
              ON gateway_usage_events(client_session_id, received_at_unix);
            "#,
        )
        .map_err(|err| err.to_string())
}

fn insert_usage_event(
    connection: &Connection,
    payload: Value,
) -> Result<GatewayUsageRecordResult, String> {
    let received_at_unix = now_unix();
    let event = GatewayUsageEvent::from_payload(&payload, received_at_unix);
    let inserted = connection
        .execute(
            r#"
            INSERT OR IGNORE INTO gateway_usage_events (
              event_id,
              emitted_at,
              received_at_unix,
              request_id,
              route_method,
              route_url,
              source_provider,
              source_adapter_key,
              target_provider,
              target_provider_name,
              target_model,
              outcome_status,
              outcome_status_code,
              error_message,
              latency_ms,
              fallback_used,
              fallback_attempts,
              identity_user_id,
              identity_tenant_id,
              client_agent_id,
              client_session_id,
              input_tokens,
              output_tokens,
              cache_read_tokens,
              cache_write_tokens,
              total_tokens,
              cache_duration_seconds,
              input_cost,
              output_cost,
              cache_read_cost,
              cache_write_cost,
              total_cost,
              currency
            )
            VALUES (
              ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
              ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28,
              ?29, ?30, ?31, ?32, ?33
            )
            "#,
            params![
                &event.event_id,
                &event.emitted_at,
                event.received_at_unix,
                &event.request_id,
                &event.route_method,
                &event.route_url,
                &event.source_provider,
                &event.source_adapter_key,
                &event.target_provider,
                &event.target_provider_name,
                &event.target_model,
                &event.outcome_status,
                event.outcome_status_code,
                &event.error_message,
                event.latency_ms,
                if event.fallback_used { 1 } else { 0 },
                event.fallback_attempts,
                &event.identity_user_id,
                &event.identity_tenant_id,
                &event.client_agent_id,
                &event.client_session_id,
                event.input_tokens,
                event.output_tokens,
                event.cache_read_tokens,
                event.cache_write_tokens,
                event.total_tokens,
                event.cache_duration_seconds,
                event.input_cost,
                event.output_cost,
                event.cache_read_cost,
                event.cache_write_cost,
                event.total_cost,
                &event.currency,
            ],
        )
        .map_err(|err| err.to_string())?
        > 0;

    Ok(GatewayUsageRecordResult {
        inserted,
        event_id: event.event_id,
    })
}

fn load_summary_from_connection(
    connection: &Connection,
    range: GatewayUsageDateRange,
    database_path: String,
    codex_home: Option<PathBuf>,
) -> Result<GatewayUsageSummary, String> {
    let totals = load_totals(connection, &range)?;
    let daily = load_daily(connection, &range)?;
    let by_provider = load_breakdown(connection, &range, BreakdownMode::Provider)?;
    let by_model = load_breakdown(connection, &range, BreakdownMode::Model)?;
    let session_ids = load_session_ids(connection, &range)?;
    let codex_sessions = load_codex_session_metadata(&session_ids, &range, codex_home.as_deref());
    let by_session = load_sessions(connection, &range, &codex_sessions)?;
    let by_project = load_projects(connection, &range, &codex_sessions)?;
    let requests = load_requests(connection, &range, &codex_sessions)?;

    Ok(GatewayUsageSummary {
        database_path,
        window_days: range.window_days,
        window_hours: range.window_hours,
        start_date: range.start_date,
        end_date: range.end_date,
        generated_at_unix: now_unix(),
        totals,
        daily,
        by_provider,
        by_model,
        by_session,
        by_project,
        requests,
    })
}

fn load_totals(
    connection: &Connection,
    range: &GatewayUsageDateRange,
) -> Result<GatewayUsageTotals, String> {
    connection
        .query_row(
            r#"
            SELECT
              COUNT(*),
              COALESCE(SUM(CASE WHEN outcome_status = 'success' THEN 1 ELSE 0 END), 0),
              COALESCE(SUM(CASE WHEN outcome_status != 'success' THEN 1 ELSE 0 END), 0),
              COALESCE(SUM(input_tokens), 0),
              COALESCE(SUM(output_tokens), 0),
              COALESCE(SUM(cache_read_tokens), 0),
              COALESCE(SUM(cache_write_tokens), 0),
              COALESCE(SUM(total_tokens), 0),
              MAX(received_at_unix)
            FROM gateway_usage_events
            WHERE received_at_unix >= ?1 AND received_at_unix < ?2
            "#,
            params![range.start_unix, range.end_unix_exclusive],
            |row| {
                Ok(GatewayUsageTotals {
                    request_count: row.get(0)?,
                    success_count: row.get(1)?,
                    error_count: row.get(2)?,
                    input_tokens: row.get(3)?,
                    output_tokens: row.get(4)?,
                    cache_read_tokens: row.get(5)?,
                    cache_write_tokens: row.get(6)?,
                    total_tokens: row.get(7)?,
                    last_received_at_unix: row.get(8)?,
                })
            },
        )
        .map_err(|err| err.to_string())
}

fn load_daily(
    connection: &Connection,
    range: &GatewayUsageDateRange,
) -> Result<Vec<GatewayUsageDaily>, String> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT
              date(received_at_unix, 'unixepoch') AS day,
              COUNT(*),
              COALESCE(SUM(input_tokens), 0),
              COALESCE(SUM(output_tokens), 0),
              COALESCE(SUM(cache_read_tokens), 0),
              COALESCE(SUM(cache_write_tokens), 0),
              COALESCE(SUM(total_tokens), 0)
            FROM gateway_usage_events
            WHERE received_at_unix >= ?1 AND received_at_unix < ?2
            GROUP BY day
            ORDER BY day ASC
            "#,
        )
        .map_err(|err| err.to_string())?;

    let rows = statement
        .query_map(params![range.start_unix, range.end_unix_exclusive], |row| {
            Ok(GatewayUsageDaily {
                day: row.get(0)?,
                request_count: row.get(1)?,
                input_tokens: row.get(2)?,
                output_tokens: row.get(3)?,
                cache_read_tokens: row.get(4)?,
                cache_write_tokens: row.get(5)?,
                total_tokens: row.get(6)?,
            })
        })
        .map_err(|err| err.to_string())?;

    collect_rows(rows)
}

#[derive(Debug, Clone, Copy)]
enum BreakdownMode {
    Provider,
    Model,
}

fn load_breakdown(
    connection: &Connection,
    range: &GatewayUsageDateRange,
    mode: BreakdownMode,
) -> Result<Vec<GatewayUsageBreakdown>, String> {
    let group_columns = match mode {
        BreakdownMode::Provider => "target_provider, target_provider_name",
        BreakdownMode::Model => "target_provider, target_provider_name, target_model",
    };
    let select_model = match mode {
        BreakdownMode::Provider => "'' AS target_model",
        BreakdownMode::Model => "target_model",
    };
    let sql = format!(
        r#"
        SELECT
          target_provider,
          target_provider_name,
          {select_model},
          COUNT(*),
          COALESCE(SUM(CASE WHEN outcome_status = 'success' THEN 1 ELSE 0 END), 0),
          COALESCE(SUM(CASE WHEN outcome_status != 'success' THEN 1 ELSE 0 END), 0),
          COALESCE(SUM(input_tokens), 0),
          COALESCE(SUM(output_tokens), 0),
          COALESCE(SUM(cache_read_tokens), 0),
          COALESCE(SUM(cache_write_tokens), 0),
          COALESCE(SUM(total_tokens), 0)
        FROM gateway_usage_events
        WHERE received_at_unix >= ?1 AND received_at_unix < ?2
        GROUP BY {group_columns}
        ORDER BY COALESCE(SUM(total_tokens), 0) DESC,
                 COUNT(*) DESC
        LIMIT 12
        "#
    );
    let mut statement = connection.prepare(&sql).map_err(|err| err.to_string())?;
    let rows = statement
        .query_map(params![range.start_unix, range.end_unix_exclusive], |row| {
            let provider: String = row.get(0)?;
            let provider_name: String = row.get(1)?;
            let model: String = row.get(2)?;
            let label = breakdown_label(&provider, &provider_name, &model, mode);
            Ok(GatewayUsageBreakdown {
                label,
                provider,
                provider_name,
                model,
                request_count: row.get(3)?,
                success_count: row.get(4)?,
                error_count: row.get(5)?,
                input_tokens: row.get(6)?,
                output_tokens: row.get(7)?,
                cache_read_tokens: row.get(8)?,
                cache_write_tokens: row.get(9)?,
                total_tokens: row.get(10)?,
            })
        })
        .map_err(|err| err.to_string())?;

    collect_rows(rows)
}

fn load_session_ids(
    connection: &Connection,
    range: &GatewayUsageDateRange,
) -> Result<HashSet<String>, String> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT DISTINCT client_session_id
            FROM gateway_usage_events
            WHERE received_at_unix >= ?1 AND received_at_unix < ?2
              AND client_session_id != ''
            "#,
        )
        .map_err(|err| err.to_string())?;

    let rows = statement
        .query_map(params![range.start_unix, range.end_unix_exclusive], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|err| err.to_string())?;

    let mut result = HashSet::new();
    for row in rows {
        let session_id = row.map_err(|err| err.to_string())?;
        if !session_id.trim().is_empty() {
            result.insert(session_id);
        }
    }
    Ok(result)
}

fn load_sessions(
    connection: &Connection,
    range: &GatewayUsageDateRange,
    codex_sessions: &CodexSessionCatalog,
) -> Result<Vec<GatewayUsageSessionBreakdown>, String> {
    load_session_breakdowns(connection, range, codex_sessions, Some(24))
}

fn load_session_breakdowns(
    connection: &Connection,
    range: &GatewayUsageDateRange,
    codex_sessions: &CodexSessionCatalog,
    limit: Option<usize>,
) -> Result<Vec<GatewayUsageSessionBreakdown>, String> {
    let mut statement = connection
        .prepare(
            r#"
        SELECT
          client_session_id,
          client_agent_id,
          received_at_unix,
          outcome_status,
          input_tokens,
          output_tokens,
          cache_read_tokens,
          cache_write_tokens,
          total_tokens
        FROM gateway_usage_events
        WHERE received_at_unix >= ?1 AND received_at_unix < ?2
        "#,
        )
        .map_err(|err| err.to_string())?;

    let rows = statement
        .query_map(params![range.start_unix, range.end_unix_exclusive], |row| {
            Ok(GatewayUsageSessionEvent {
                client_session_id: row.get(0)?,
                client_agent_id: row.get(1)?,
                received_at_unix: row.get(2)?,
                outcome_status: row.get(3)?,
                input_tokens: row.get(4)?,
                output_tokens: row.get(5)?,
                cache_read_tokens: row.get(6)?,
                cache_write_tokens: row.get(7)?,
                total_tokens: row.get(8)?,
            })
        })
        .map_err(|err| err.to_string())?;

    let mut sessions: HashMap<String, GatewayUsageSessionBreakdown> = HashMap::new();
    for row in rows {
        let event = row.map_err(|err| err.to_string())?;
        let (session_id, metadata) = resolve_codex_session_for_event(
            &event.client_session_id,
            &event.client_agent_id,
            event.received_at_unix,
            codex_sessions,
        );
        let project_path = metadata
            .map(|value| value.project_path.clone())
            .unwrap_or_default();
        let entry =
            sessions
                .entry(session_id.clone())
                .or_insert_with(|| GatewayUsageSessionBreakdown {
                    label: session_label(&session_id, metadata),
                    project_label: project_label(&project_path),
                    project_path: project_path.clone(),
                    session_id: session_id.clone(),
                    request_count: 0,
                    success_count: 0,
                    error_count: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    total_tokens: 0,
                    first_received_at_unix: None,
                    last_received_at_unix: None,
                });
        entry.request_count += 1;
        if event.outcome_status == "success" {
            entry.success_count += 1;
        } else {
            entry.error_count += 1;
        }
        entry.input_tokens += event.input_tokens;
        entry.output_tokens += event.output_tokens;
        entry.cache_read_tokens += event.cache_read_tokens;
        entry.cache_write_tokens += event.cache_write_tokens;
        entry.total_tokens += event.total_tokens;
        entry.first_received_at_unix =
            min_optional_unix(entry.first_received_at_unix, Some(event.received_at_unix));
        entry.last_received_at_unix =
            max_optional_unix(entry.last_received_at_unix, Some(event.received_at_unix));
    }

    let mut result: Vec<GatewayUsageSessionBreakdown> = sessions.into_values().collect();
    result.sort_by(|left, right| {
        right
            .total_tokens
            .cmp(&left.total_tokens)
            .then_with(|| right.request_count.cmp(&left.request_count))
            .then_with(|| left.label.cmp(&right.label))
    });
    if let Some(limit) = limit {
        result.truncate(limit);
    }
    Ok(result)
}

fn load_projects(
    connection: &Connection,
    range: &GatewayUsageDateRange,
    codex_sessions: &CodexSessionCatalog,
) -> Result<Vec<GatewayUsageProjectBreakdown>, String> {
    let sessions = load_session_breakdowns(connection, range, codex_sessions, None)?;
    let mut projects: HashMap<String, GatewayUsageProjectAccumulator> = HashMap::new();

    for session in sessions {
        let key = session.project_path.clone();
        projects
            .entry(key.clone())
            .or_insert_with(|| GatewayUsageProjectAccumulator::new(key))
            .add_session(&session);
    }

    let mut result: Vec<GatewayUsageProjectBreakdown> = projects
        .into_values()
        .map(GatewayUsageProjectAccumulator::into_breakdown)
        .collect();
    result.sort_by(|left, right| {
        right
            .total_tokens
            .cmp(&left.total_tokens)
            .then_with(|| right.request_count.cmp(&left.request_count))
            .then_with(|| left.label.cmp(&right.label))
    });
    result.truncate(24);
    Ok(result)
}

fn load_requests(
    connection: &Connection,
    range: &GatewayUsageDateRange,
    codex_sessions: &CodexSessionCatalog,
) -> Result<Vec<GatewayUsageRequestEvent>, String> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT
              event_id,
              request_id,
              emitted_at,
              received_at_unix,
              client_session_id,
              client_agent_id,
              route_method,
              route_url,
              target_provider,
              target_provider_name,
              target_model,
              outcome_status,
              outcome_status_code,
              latency_ms,
              input_tokens,
              output_tokens,
              cache_read_tokens,
              cache_write_tokens,
              total_tokens
            FROM gateway_usage_events
            WHERE received_at_unix >= ?1 AND received_at_unix < ?2
            ORDER BY received_at_unix DESC, id DESC
            "#,
        )
        .map_err(|err| err.to_string())?;

    let rows = statement
        .query_map(params![range.start_unix, range.end_unix_exclusive], |row| {
            let session_id: String = row.get(4)?;
            let agent_id: String = row.get(5)?;
            let received_at_unix = row.get(3)?;
            let (session_id, metadata) = resolve_codex_session_for_event(
                &session_id,
                &agent_id,
                received_at_unix,
                codex_sessions,
            );
            let method: String = row.get(6)?;
            let url: String = row.get(7)?;
            let project_path = metadata
                .map(|value| value.project_path.clone())
                .unwrap_or_default();
            Ok(GatewayUsageRequestEvent {
                event_id: row.get(0)?,
                request_id: row.get(1)?,
                emitted_at: row.get(2)?,
                received_at_unix,
                client_session_label: request_session_label(metadata),
                client_project_label: project_label(&project_path),
                client_project_path: project_path,
                client_session_id: session_id,
                route: route_label(&method, &url),
                provider: row.get(8)?,
                provider_name: row.get(9)?,
                model: row.get(10)?,
                status: row.get(11)?,
                status_code: row.get(12)?,
                latency_ms: row.get(13)?,
                input_tokens: row.get(14)?,
                output_tokens: row.get(15)?,
                cache_read_tokens: row.get(16)?,
                cache_write_tokens: row.get(17)?,
                total_tokens: row.get(18)?,
            })
        })
        .map_err(|err| err.to_string())?;

    collect_rows(rows)
}

fn collect_rows<T, F>(rows: rusqlite::MappedRows<'_, F>) -> Result<Vec<T>, String>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|err| err.to_string())?);
    }
    Ok(result)
}

#[derive(Debug)]
struct GatewayUsageSessionEvent {
    client_session_id: String,
    client_agent_id: String,
    received_at_unix: i64,
    outcome_status: String,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    total_tokens: i64,
}

#[derive(Debug, Default)]
struct GatewayUsageProjectAccumulator {
    project_path: String,
    session_ids: HashSet<String>,
    request_count: i64,
    success_count: i64,
    error_count: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    total_tokens: i64,
    first_received_at_unix: Option<i64>,
    last_received_at_unix: Option<i64>,
}

impl GatewayUsageProjectAccumulator {
    fn new(project_path: String) -> Self {
        Self {
            project_path,
            ..Self::default()
        }
    }

    fn add_session(&mut self, session: &GatewayUsageSessionBreakdown) {
        if !session.session_id.trim().is_empty() {
            self.session_ids.insert(session.session_id.clone());
        }
        self.request_count += session.request_count;
        self.success_count += session.success_count;
        self.error_count += session.error_count;
        self.input_tokens += session.input_tokens;
        self.output_tokens += session.output_tokens;
        self.cache_read_tokens += session.cache_read_tokens;
        self.cache_write_tokens += session.cache_write_tokens;
        self.total_tokens += session.total_tokens;
        self.first_received_at_unix =
            min_optional_unix(self.first_received_at_unix, session.first_received_at_unix);
        self.last_received_at_unix =
            max_optional_unix(self.last_received_at_unix, session.last_received_at_unix);
    }

    fn into_breakdown(self) -> GatewayUsageProjectBreakdown {
        GatewayUsageProjectBreakdown {
            label: project_label(&self.project_path),
            project_path: self.project_path,
            session_count: self.session_ids.len() as i64,
            request_count: self.request_count,
            success_count: self.success_count,
            error_count: self.error_count,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_read_tokens: self.cache_read_tokens,
            cache_write_tokens: self.cache_write_tokens,
            total_tokens: self.total_tokens,
            first_received_at_unix: self.first_received_at_unix,
            last_received_at_unix: self.last_received_at_unix,
        }
    }
}

fn min_optional_unix(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn max_optional_unix(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn load_codex_session_metadata(
    session_ids: &HashSet<String>,
    range: &GatewayUsageDateRange,
    codex_home: Option<&Path>,
) -> CodexSessionCatalog {
    if let Some(codex_home) = codex_home {
        return load_codex_session_metadata_from_home(codex_home, session_ids, range);
    }
    load_codex_session_metadata_from_home(&codex_home_dir(), session_ids, range)
}

fn load_codex_session_metadata_from_home(
    codex_home: &Path,
    session_ids: &HashSet<String>,
    range: &GatewayUsageDateRange,
) -> CodexSessionCatalog {
    let index = load_codex_session_index(codex_home);
    let mut catalog = CodexSessionCatalog::default();
    load_codex_session_files(
        &codex_home.join("sessions"),
        session_ids,
        range,
        &index,
        &mut catalog,
    );

    for session_id in session_ids {
        if session_id.trim().is_empty() || catalog.by_id.contains_key(session_id) {
            continue;
        }
        if let Some(index_entry) = index.get(session_id) {
            catalog.by_id.insert(
                session_id.clone(),
                CodexSessionMetadata {
                    id: session_id.clone(),
                    title: index_entry.title.clone(),
                    last_seen_at_unix: index_entry.updated_at_unix,
                    ..CodexSessionMetadata::default()
                },
            );
        }
    }

    catalog.timeline.sort_by(|left, right| {
        left.started_at_unix
            .cmp(&right.started_at_unix)
            .then_with(|| left.id.cmp(&right.id))
    });
    catalog
}

fn load_codex_session_index(codex_home: &Path) -> HashMap<String, CodexSessionIndexEntry> {
    let mut result = HashMap::new();
    let Ok(file) = File::open(codex_home.join("session_index.jsonl")) else {
        return result;
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(session_id) = read_string(&value, &["id"]) else {
            continue;
        };
        let title = read_first_string(
            &value,
            &[&["thread_name"], &["name"], &["title"], &["threadName"]],
        )
        .unwrap_or_default();
        let updated_at_unix =
            read_string(&value, &["updated_at"]).and_then(|value| parse_rfc3339_unix(&value));
        result.insert(
            session_id,
            CodexSessionIndexEntry {
                title,
                updated_at_unix,
            },
        );
    }
    result
}

fn load_codex_session_files(
    dir: &Path,
    session_ids: &HashSet<String>,
    range: &GatewayUsageDateRange,
    index: &HashMap<String, CodexSessionIndexEntry>,
    catalog: &mut CodexSessionCatalog,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            load_codex_session_files(&path, session_ids, range, index, catalog);
            continue;
        }
        let Some(mut metadata) = read_codex_session_metadata_from_file(&path, index) else {
            continue;
        };
        if !metadata_overlaps_usage_range(&metadata, range)
            && !session_ids.contains(&metadata.id)
            && !path_matches_any_session_id(&path, session_ids)
        {
            continue;
        }

        if let Some(index_entry) = index.get(&metadata.id) {
            if metadata.title.is_empty() {
                metadata.title = index_entry.title.clone();
            }
            metadata.last_seen_at_unix =
                max_optional_unix(metadata.last_seen_at_unix, index_entry.updated_at_unix);
        }
        metadata.last_seen_at_unix =
            max_optional_unix(metadata.last_seen_at_unix, metadata.started_at_unix);
        if metadata.id.trim().is_empty() {
            continue;
        }

        catalog
            .by_id
            .entry(metadata.id.clone())
            .or_insert_with(|| metadata.clone());
        if path_matches_any_session_id(&path, session_ids) {
            for session_id in session_ids {
                if !session_id.trim().is_empty()
                    && path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|file_name| file_name.contains(session_id))
                {
                    catalog.by_id.insert(session_id.clone(), metadata.clone());
                }
            }
        }
        catalog.timeline.push(metadata);
    }
}

fn read_codex_session_metadata_from_file(
    path: &Path,
    index: &HashMap<String, CodexSessionIndexEntry>,
) -> Option<CodexSessionMetadata> {
    let file = File::open(path).ok()?;
    let mut metadata = CodexSessionMetadata {
        id: codex_session_id_from_file_name(path).unwrap_or_default(),
        last_seen_at_unix: file_modified_unix(path),
        ..CodexSessionMetadata::default()
    };
    for line in BufReader::new(file).lines().map_while(Result::ok).take(48) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if metadata.id.is_empty() {
            metadata.id =
                read_first_string(&value, &[&["payload", "id"], &["id"]]).unwrap_or_default();
        }
        if metadata.project_path.is_empty() {
            metadata.project_path = read_first_string(
                &value,
                &[
                    &["payload", "cwd"],
                    &["cwd"],
                    &["payload", "workspacePath"],
                    &["payload", "workspace_path"],
                    &["workspacePath"],
                    &["workspace_path"],
                ],
            )
            .unwrap_or_default();
        }
        if metadata.started_at_unix.is_none() {
            metadata.started_at_unix = read_first_string(
                &value,
                &[&["payload", "timestamp"], &["timestamp"], &["created_at"]],
            )
            .and_then(|value| parse_rfc3339_unix(&value));
        }
        if !metadata.is_subagent {
            metadata.is_subagent = read_first_string(&value, &[&["payload", "thread_source"]])
                .is_some_and(|value| value == "subagent");
        }
        if !metadata.id.is_empty()
            && !metadata.project_path.is_empty()
            && metadata.started_at_unix.is_some()
        {
            break;
        }
    }

    if metadata.id.is_empty() {
        return None;
    }
    if let Some(index_entry) = index.get(&metadata.id) {
        metadata.title = index_entry.title.clone();
        metadata.last_seen_at_unix =
            max_optional_unix(metadata.last_seen_at_unix, index_entry.updated_at_unix);
    }
    Some(metadata)
}

fn resolve_codex_session_for_event<'a>(
    session_id: &str,
    agent_id: &str,
    received_at_unix: i64,
    catalog: &'a CodexSessionCatalog,
) -> (String, Option<&'a CodexSessionMetadata>) {
    let session_id = session_id.trim();
    if !session_id.is_empty() {
        if let Some(metadata) = catalog.by_id.get(session_id) {
            return (metadata.id.clone(), Some(metadata));
        }
    }

    if is_codex_agent(agent_id) && should_infer_codex_session_from_raw_session_id(session_id) {
        if let Some(metadata) = infer_codex_session_at(catalog, received_at_unix) {
            return (metadata.id.clone(), Some(metadata));
        }
    }

    (session_id.to_string(), None)
}

fn infer_codex_session_at(
    catalog: &CodexSessionCatalog,
    received_at_unix: i64,
) -> Option<&CodexSessionMetadata> {
    catalog
        .timeline
        .iter()
        .filter_map(|metadata| {
            let distance = codex_session_time_distance(metadata, received_at_unix)?;
            (distance <= 6 * 3_600).then_some((metadata, distance))
        })
        .min_by(|(left, left_distance), (right, right_distance)| {
            left_distance
                .cmp(right_distance)
                .then_with(|| left.is_subagent.cmp(&right.is_subagent))
                .then_with(|| {
                    right
                        .started_at_unix
                        .unwrap_or_default()
                        .cmp(&left.started_at_unix.unwrap_or_default())
                })
                .then_with(|| left.id.cmp(&right.id))
        })
        .map(|(metadata, _)| metadata)
}

fn codex_session_time_distance(
    metadata: &CodexSessionMetadata,
    received_at_unix: i64,
) -> Option<i64> {
    let start = metadata.started_at_unix?;
    let end = metadata.last_seen_at_unix.unwrap_or(start).max(start);
    if received_at_unix < start {
        Some(start - received_at_unix)
    } else if received_at_unix > end {
        Some(received_at_unix - end)
    } else {
        Some(0)
    }
}

fn is_codex_agent(agent_id: &str) -> bool {
    agent_id.trim().to_ascii_lowercase().contains("codex")
}

fn should_infer_codex_session_from_raw_session_id(session_id: &str) -> bool {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return true;
    }
    let normalized = session_id.to_ascii_lowercase();
    normalized.starts_with("resp_")
        || normalized.starts_with("response_")
        || normalized.starts_with("chatcmpl-")
}

fn metadata_overlaps_usage_range(
    metadata: &CodexSessionMetadata,
    range: &GatewayUsageDateRange,
) -> bool {
    let start = metadata.started_at_unix.unwrap_or(i64::MIN / 4);
    let end = metadata.last_seen_at_unix.unwrap_or(start).max(start);
    end >= range.start_unix - 6 * 3_600 && start < range.end_unix_exclusive + 6 * 3_600
}

fn path_matches_any_session_id(path: &Path, session_ids: &HashSet<String>) -> bool {
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    session_ids
        .iter()
        .any(|session_id| !session_id.trim().is_empty() && file_name.contains(session_id))
}

fn codex_session_id_from_file_name(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    let without_prefix = file_name.strip_prefix("rollout-")?;
    let without_suffix = without_prefix.strip_suffix(".jsonl")?;
    (without_suffix.len() >= 36).then(|| without_suffix[without_suffix.len() - 36..].to_string())
}

fn file_modified_unix(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs() as i64)
}

fn parse_rfc3339_unix(value: &str) -> Option<i64> {
    let trimmed = value.trim();
    let (date, time) = trimmed.split_once('T')?;
    let mut date_parts = date.split('-');
    let year = date_parts.next()?.parse::<i32>().ok()?;
    let month = date_parts.next()?.parse::<u32>().ok()?;
    let day = date_parts.next()?.parse::<u32>().ok()?;
    let time = time.trim_end_matches('Z');
    let time = time.split_once('+').map(|(value, _)| value).unwrap_or(time);
    let time = time.split_once('-').map(|(value, _)| value).unwrap_or(time);
    let mut time_parts = time.split(':');
    let hour = time_parts.next()?.parse::<i64>().ok()?;
    let minute = time_parts.next()?.parse::<i64>().ok()?;
    let second_text = time_parts.next()?;
    let second = second_text
        .split_once('.')
        .map(|(value, _)| value)
        .unwrap_or(second_text)
        .parse::<i64>()
        .ok()?;
    let day_number = days_from_civil(year, month, day).ok()?;
    Some(day_number * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn read_first_string(value: &Value, paths: &[&[&str]]) -> Option<String> {
    paths.iter().find_map(|path| {
        read_string(value, path).and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
    })
}

fn codex_home_dir() -> PathBuf {
    if let Ok(value) = std::env::var("CODEX_HOME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    std::env::var("HOME")
        .map(|value| PathBuf::from(value).join(".codex"))
        .unwrap_or_else(|_| PathBuf::from(".codex"))
}

impl GatewayUsageEvent {
    fn from_payload(payload: &Value, received_at_unix: i64) -> Self {
        let emitted_at = read_string(payload, &["emittedAt"]).unwrap_or_default();
        let request_id = read_string(payload, &["requestId"]).unwrap_or_default();
        let fallback_attempts = read_i64(payload, &["fallback", "attempts"]).unwrap_or(0);
        let input_tokens = read_i64(payload, &["billing", "usage", "input_tokens"]).unwrap_or(0);
        let output_tokens = read_i64(payload, &["billing", "usage", "output_tokens"]).unwrap_or(0);
        let cache_read_tokens =
            read_i64(payload, &["billing", "usage", "cache_read_tokens"]).unwrap_or(0);
        let cache_write_tokens =
            read_i64(payload, &["billing", "usage", "cache_write_tokens"]).unwrap_or(0);
        let total_tokens = read_i64(payload, &["billing", "usage", "total_tokens"])
            .filter(|value| *value > 0)
            .unwrap_or(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens);
        let status_code = read_i64(payload, &["outcome", "statusCode"]);
        let outcome_status = read_string(payload, &["outcome", "status"])
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| outcome_status_from_code(status_code));
        let event_id = read_string(payload, &["eventId"])
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| fallback_event_id(&request_id, received_at_unix));

        Self {
            event_id,
            emitted_at,
            received_at_unix,
            request_id,
            route_method: read_string(payload, &["route", "method"]).unwrap_or_default(),
            route_url: read_string(payload, &["route", "url"]).unwrap_or_default(),
            source_provider: read_string(payload, &["source", "provider"]).unwrap_or_default(),
            source_adapter_key: read_string(payload, &["source", "adapterKey"]).unwrap_or_default(),
            target_provider: read_string(payload, &["target", "provider"])
                .or_else(|| read_string(payload, &["billing", "provider"]))
                .unwrap_or_default(),
            target_provider_name: read_string(payload, &["target", "providerName"])
                .unwrap_or_default(),
            target_model: read_string(payload, &["target", "model"]).unwrap_or_default(),
            outcome_status,
            outcome_status_code: status_code,
            error_message: read_string(payload, &["outcome", "errorMessage"]).unwrap_or_default(),
            latency_ms: read_i64(payload, &["performance", "latency_ms"]),
            fallback_used: read_bool(payload, &["fallback", "used"])
                .unwrap_or(fallback_attempts > 0),
            fallback_attempts,
            identity_user_id: read_string(payload, &["identity", "userId"]).unwrap_or_default(),
            identity_tenant_id: read_string(payload, &["identity", "tenantId"]).unwrap_or_default(),
            client_agent_id: read_string(payload, &["clientContext", "agentId"])
                .unwrap_or_default(),
            client_session_id: read_string(payload, &["clientContext", "sessionId"])
                .unwrap_or_default(),
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            total_tokens,
            cache_duration_seconds: read_i64(
                payload,
                &["billing", "usage", "cache_duration_seconds"],
            )
            .unwrap_or(0),
            input_cost: 0.0,
            output_cost: 0.0,
            cache_read_cost: 0.0,
            cache_write_cost: 0.0,
            total_cost: 0.0,
            currency: read_string(payload, &["billing", "currency"])
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "USD".to_string()),
        }
    }
}

fn normalize_window_days(days: Option<u32>) -> i64 {
    i64::from(
        days.unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS),
    )
}

fn normalize_window_hours(hours: Option<u32>) -> i64 {
    i64::from(
        hours
            .unwrap_or(DEFAULT_WINDOW_HOURS)
            .clamp(1, MAX_WINDOW_HOURS),
    )
}

fn normalize_date_range(
    days: Option<u32>,
    start_date: Option<String>,
    end_date: Option<String>,
    hours: Option<u32>,
) -> Result<GatewayUsageDateRange, String> {
    let start_input = date_input(start_date);
    let end_input = date_input(end_date);
    if hours.is_some() || (days.is_none() && start_input.is_none() && end_input.is_none()) {
        let window_hours = normalize_window_hours(hours);
        let end_unix_exclusive = now_unix() + 1;
        let start_unix = end_unix_exclusive - window_hours * 3_600;
        let start_day = start_unix.div_euclid(86_400);
        let end_day = (end_unix_exclusive - 1).div_euclid(86_400);
        let span_days = end_day - start_day + 1;
        return Ok(GatewayUsageDateRange {
            window_days: span_days as u32,
            window_hours: window_hours as u32,
            start_date: day_to_date_string(start_day),
            end_date: day_to_date_string(end_day),
            start_unix,
            end_unix_exclusive,
        });
    }

    let window_days = normalize_window_days(days);
    let today_day = now_unix().div_euclid(86_400);
    let end_day = match end_input {
        Some(value) => parse_date_day(&value)?,
        None => today_day,
    };
    let start_day = match start_input {
        Some(value) => parse_date_day(&value)?,
        None => end_day - window_days + 1,
    };

    if start_day > end_day {
        return Err("Gateway usage start date must be on or before the end date".to_string());
    }
    let span_days = end_day - start_day + 1;
    if span_days > i64::from(MAX_WINDOW_DAYS) {
        return Err(format!(
            "Gateway usage date range cannot exceed {} days",
            MAX_WINDOW_DAYS
        ));
    }

    Ok(GatewayUsageDateRange {
        window_days: span_days as u32,
        window_hours: (span_days as u32) * 24,
        start_date: day_to_date_string(start_day),
        end_date: day_to_date_string(end_day),
        start_unix: start_day * 86_400,
        end_unix_exclusive: (end_day + 1) * 86_400,
    })
}

fn date_input(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_date_day(value: &str) -> Result<i64, String> {
    let mut parts = value.split('-');
    let year = parts
        .next()
        .and_then(|part| part.parse::<i32>().ok())
        .ok_or_else(|| format!("Invalid Gateway usage date: {}", value))?;
    let month = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .ok_or_else(|| format!("Invalid Gateway usage date: {}", value))?;
    let day = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .ok_or_else(|| format!("Invalid Gateway usage date: {}", value))?;
    if parts.next().is_some() {
        return Err(format!("Invalid Gateway usage date: {}", value));
    }
    let day_number = days_from_civil(year, month, day)?;
    if day_to_date_string(day_number) != value {
        return Err(format!("Invalid Gateway usage date: {}", value));
    }
    Ok(day_number)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Result<i64, String> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err("Invalid Gateway usage date".to_string());
    }
    let year = year - if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Ok(i64::from(era * 146_097 + doe - 719_468))
}

fn day_to_date_string(day_number: i64) -> String {
    let (year, month, day) = civil_from_days(day_number);
    format!("{:04}-{:02}-{:02}", year, month, day)
}

fn civil_from_days(day_number: i64) -> (i64, i64, i64) {
    let day_number = day_number + 719_468;
    let era = if day_number >= 0 {
        day_number
    } else {
        day_number - 146_096
    } / 146_097;
    let doe = day_number - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

fn database_path() -> PathBuf {
    std::env::var("CODEXL_GATEWAY_USAGE_DB_PATH")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(crate::extensions::builtins::expand_home_path)
        .unwrap_or_else(|| {
            crate::extensions::builtins::codexl_home_dir().join("gateway-usage.sqlite3")
        })
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn fallback_event_id(request_id: &str, received_at_unix: i64) -> String {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let request = request_id.trim();
    if request.is_empty() {
        format!("local-{}-{}", received_at_unix, suffix)
    } else {
        format!("{}-{}", request, suffix)
    }
}

fn outcome_status_from_code(status_code: Option<i64>) -> String {
    match status_code {
        Some(200..=399) | None => "success".to_string(),
        Some(408 | 504) => "timeout".to_string(),
        Some(429) => "rate-limited".to_string(),
        Some(_) => "error".to_string(),
    }
}

fn breakdown_label(
    provider: &str,
    provider_name: &str,
    model: &str,
    mode: BreakdownMode,
) -> String {
    let provider_label = if provider_name.trim().is_empty() {
        provider.trim()
    } else {
        provider_name.trim()
    };
    let provider_label = if provider_label.is_empty() {
        "unknown"
    } else {
        provider_label
    };
    match mode {
        BreakdownMode::Provider => provider_label.to_string(),
        BreakdownMode::Model => {
            let model = model.trim();
            if model.is_empty() {
                provider_label.to_string()
            } else if model.contains('/') {
                model.to_string()
            } else {
                format!("{}/{}", provider_label, model)
            }
        }
    }
}

fn session_label(session_id: &str, metadata: Option<&CodexSessionMetadata>) -> String {
    if let Some(title) = metadata
        .map(|value| value.title.trim())
        .filter(|value| !value.is_empty())
    {
        return title.to_string();
    }
    let session_id = session_id.trim();
    if session_id.is_empty() {
        "unknown session".to_string()
    } else {
        session_id.to_string()
    }
}

fn request_session_label(metadata: Option<&CodexSessionMetadata>) -> String {
    metadata
        .map(|value| value.title.trim())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| "-".to_string())
}

fn project_label(project_path: &str) -> String {
    let project_path = project_path.trim();
    if project_path.is_empty() {
        return "unknown project".to_string();
    }
    Path::new(project_path)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(project_path)
        .to_string()
}

fn route_label(method: &str, url: &str) -> String {
    let method = method.trim();
    let url = url.trim();
    if method.is_empty() {
        url.to_string()
    } else if url.is_empty() {
        method.to_string()
    } else {
        format!("{} {}", method, url)
    }
}

fn read_value<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn read_string(value: &Value, path: &[&str]) -> Option<String> {
    let value = read_value(value, path)?;
    if let Some(text) = value.as_str() {
        return Some(text.trim().to_string());
    }
    if value.is_number() || value.is_boolean() {
        return Some(value.to_string());
    }
    None
}

fn read_i64(value: &Value, path: &[&str]) -> Option<i64> {
    let value = read_value(value, path)?;
    if let Some(value) = value.as_i64() {
        return Some(value);
    }
    if let Some(value) = value.as_u64() {
        return i64::try_from(value).ok();
    }
    if let Some(value) = value.as_f64() {
        if value.is_finite() {
            return Some(value.trunc() as i64);
        }
    }
    value.as_str()?.trim().parse::<i64>().ok()
}

fn read_bool(value: &Value, path: &[&str]) -> Option<bool> {
    let value = read_value(value, path)?;
    if let Some(value) = value.as_bool() {
        return Some(value);
    }
    if let Some(value) = value.as_i64() {
        return Some(value != 0);
    }
    match value.as_str()?.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_usage_event_from_gateway_billing_payload() {
        let payload = json!({
            "eventId": "evt_1",
            "emittedAt": "2026-05-27T12:00:00.000Z",
            "requestId": "req_1",
            "route": { "method": "POST", "url": "/v1/responses" },
            "source": { "provider": "openai", "adapterKey": "openai_responses" },
            "target": { "provider": "openai", "providerName": "primary", "model": "gpt-4.1" },
            "fallback": { "used": false, "attempts": 0 },
            "performance": { "latency_ms": 1234 },
            "outcome": { "status": "success", "statusCode": 200 },
            "clientContext": { "agentId": "codex", "sessionId": "session-a" },
            "billing": {
                "provider": "openai",
                "currency": "USD",
                "usage": {
                    "input_tokens": 100,
                    "output_tokens": 40,
                    "cache_read_tokens": 10,
                    "total_tokens": 150
                },
                "cost": { "input": 0.01, "output": 0.02, "total": 0.03 }
            }
        });

        let event = GatewayUsageEvent::from_payload(&payload, 1_779_873_600);

        assert_eq!(event.event_id, "evt_1");
        assert_eq!(event.target_provider_name, "primary");
        assert_eq!(event.target_model, "gpt-4.1");
        assert_eq!(event.input_tokens, 100);
        assert_eq!(event.output_tokens, 40);
        assert_eq!(event.cache_read_tokens, 10);
        assert_eq!(event.total_tokens, 150);
        assert_eq!(event.latency_ms, Some(1234));
        assert_eq!(event.client_session_id, "session-a");
    }

    #[test]
    fn aggregates_usage_summary_from_sqlite() {
        let connection = Connection::open_in_memory().expect("open sqlite");
        init_database(&connection).expect("init database");
        insert_usage_event(
            &connection,
            json!({
                "eventId": "evt_1",
                "requestId": "req_1",
                "target": { "provider": "openai", "providerName": "primary", "model": "gpt-4.1" },
                "outcome": { "status": "success", "statusCode": 200 },
                "clientContext": { "agentId": "codex", "sessionId": "session-a" },
                "billing": {
                    "provider": "openai",
                    "usage": { "input_tokens": 10, "output_tokens": 20, "cache_read_tokens": 4, "total_tokens": 34 },
                    "cost": { "total": 0.12 }
                }
            }),
        )
        .expect("insert first event");
        insert_usage_event(
            &connection,
            json!({
                "eventId": "evt_2",
                "requestId": "req_2",
                "target": { "provider": "anthropic", "providerName": "backup", "model": "claude" },
                "outcome": { "status": "error", "statusCode": 500 },
                "clientContext": { "agentId": "codex", "sessionId": "session-a" },
                "billing": {
                    "provider": "anthropic",
                    "usage": { "input_tokens": 5, "output_tokens": 0, "total_tokens": 5 },
                    "cost": { "total": 0.03 }
                }
            }),
        )
        .expect("insert second event");

        let range = normalize_date_range(Some(30), None, None, None).expect("normalize date range");
        let summary =
            load_summary_from_connection(&connection, range, ":memory:".to_string(), None)
                .expect("load summary");

        assert_eq!(summary.totals.request_count, 2);
        assert_eq!(summary.totals.success_count, 1);
        assert_eq!(summary.totals.error_count, 1);
        assert_eq!(summary.totals.total_tokens, 39);
        assert_eq!(summary.totals.cache_read_tokens, 4);
        assert_eq!(summary.by_model.len(), 2);
        assert_eq!(summary.by_session.len(), 1);
        assert_eq!(summary.by_session[0].session_id, "session-a");
        assert_eq!(summary.by_session[0].request_count, 2);
        assert_eq!(summary.by_project.len(), 1);
        assert_eq!(summary.by_project[0].session_count, 1);
        assert_eq!(summary.by_project[0].request_count, 2);
        assert_eq!(summary.requests.len(), 2);
        assert_eq!(summary.requests[0].client_session_id, "session-a");
        assert_eq!(summary.requests[0].client_session_label, "-");
    }

    #[test]
    fn loads_codex_session_titles_and_projects_from_local_records() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let codex_home = std::env::temp_dir().join(format!("codexl-gateway-usage-test-{}", suffix));
        let session_dir = codex_home
            .join("sessions")
            .join("2026")
            .join("05")
            .join("27");
        std::fs::create_dir_all(&session_dir).expect("create codex session dir");
        std::fs::write(
            codex_home.join("session_index.jsonl"),
            r#"{"id":"session-a","thread_name":"Gateway Usage Dashboard","updated_at":"2026-05-27T12:00:00Z"}
{"id":"session-b","thread_name":"Other Session","updated_at":"2026-05-27T12:01:00Z"}
"#,
        )
        .expect("write session index");
        std::fs::write(
            session_dir.join("rollout-2026-05-27T12-00-00-session-a.jsonl"),
            r#"{"timestamp":"2026-05-27T12:00:00Z","type":"session_meta","payload":{"id":"session-a","cwd":"/Users/jinhuilee/products/CodexL"}}"#,
        )
        .expect("write codex session");

        let mut session_ids = HashSet::new();
        session_ids.insert("session-a".to_string());
        let range = GatewayUsageDateRange {
            window_days: 1,
            window_hours: 24,
            start_date: "2026-05-27".to_string(),
            end_date: "2026-05-27".to_string(),
            start_unix: parse_rfc3339_unix("2026-05-27T00:00:00Z").expect("start"),
            end_unix_exclusive: parse_rfc3339_unix("2026-05-28T00:00:00Z").expect("end"),
        };
        let catalog = load_codex_session_metadata_from_home(&codex_home, &session_ids, &range);

        let session = catalog.by_id.get("session-a").expect("session metadata");
        assert_eq!(session.title, "Gateway Usage Dashboard");
        assert_eq!(session.project_path, "/Users/jinhuilee/products/CodexL");

        let _ = std::fs::remove_dir_all(codex_home);
    }

    #[test]
    fn usage_summary_uses_configured_codex_home_for_request_session_titles() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let codex_home =
            std::env::temp_dir().join(format!("codexl-gateway-usage-config-home-test-{}", suffix));
        let session_dir = codex_home
            .join("sessions")
            .join("2026")
            .join("05")
            .join("27");
        std::fs::create_dir_all(&session_dir).expect("create codex session dir");
        std::fs::write(
            codex_home.join("session_index.jsonl"),
            r#"{"id":"session-a","thread_name":"Configured Home Session","updated_at":"2026-05-27T12:00:00Z"}
"#,
        )
        .expect("write session index");
        std::fs::write(
            session_dir.join("rollout-2026-05-27T12-00-00-session-a.jsonl"),
            r#"{"timestamp":"2026-05-27T12:00:00Z","type":"session_meta","payload":{"id":"session-a","cwd":"/tmp/configured-home"}}"#,
        )
        .expect("write codex session");

        let connection = Connection::open_in_memory().expect("open sqlite");
        init_database(&connection).expect("init database");
        insert_usage_event(
            &connection,
            json!({
                "eventId": "evt_1",
                "requestId": "req_1",
                "target": { "provider": "openai", "providerName": "primary", "model": "gpt-4.1" },
                "outcome": { "status": "success", "statusCode": 200 },
                "clientContext": { "agentId": "codex", "sessionId": "session-a" },
                "billing": {
                    "provider": "openai",
                    "usage": { "input_tokens": 10, "output_tokens": 20, "total_tokens": 30 }
                }
            }),
        )
        .expect("insert event");

        let range = normalize_date_range(Some(30), None, None, None).expect("normalize date range");
        let summary = load_summary_from_connection(
            &connection,
            range,
            ":memory:".to_string(),
            Some(codex_home.clone()),
        )
        .expect("load summary");

        assert_eq!(summary.requests[0].client_session_id, "session-a");
        assert_eq!(
            summary.requests[0].client_session_label,
            "Configured Home Session"
        );
        assert_eq!(summary.requests[0].client_project_label, "configured-home");

        let _ = std::fs::remove_dir_all(codex_home);
    }

    #[test]
    fn infers_empty_codex_gateway_session_from_event_time() {
        let metadata = CodexSessionMetadata {
            id: "session-a".to_string(),
            title: "Gateway Usage Dashboard".to_string(),
            project_path: "/Users/jinhuilee/products/CodexL".to_string(),
            started_at_unix: Some(100),
            last_seen_at_unix: Some(200),
            is_subagent: false,
        };
        let mut catalog = CodexSessionCatalog::default();
        catalog.by_id.insert(metadata.id.clone(), metadata.clone());
        catalog.timeline.push(metadata);

        let (session_id, session) = resolve_codex_session_for_event("", "codex", 150, &catalog);

        assert_eq!(session_id, "session-a");
        assert_eq!(session.expect("metadata").title, "Gateway Usage Dashboard");
    }
}
