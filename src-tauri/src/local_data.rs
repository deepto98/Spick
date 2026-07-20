//! Native-owned local usage, history, and vocabulary storage.
//!
//! The webviews never open SQLite directly. A database failure disables these
//! optional product features for the process, but it must never turn a valid
//! transcription or insertion into a failed/retried dictation.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::{Days, Local, LocalResult, NaiveDate, TimeZone};
use rusqlite::{
    params, Connection, Error as SqlError, ErrorCode, OpenFlags, OptionalExtension,
    TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::domain::DictationDeliveryStatus;

const SCHEMA_VERSION: i64 = 1;
const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_HISTORY_LIMIT: u16 = 25;
const MAX_HISTORY_LIMIT: u16 = 100;
const MAX_VOCABULARY_ENTRIES: usize = 64;
const MAX_VOCABULARY_PROMPT_BYTES: usize = 1_024;
const MAX_PHRASE_GRAPHEMES: usize = 120;
const MAX_PHRASE_BYTES: usize = 512;
const MAX_LANGUAGE_TAG_BYTES: usize = 35;
const TYPING_BASELINE_WPM: u16 = 40;

pub const LOCAL_DATA_CHANGED_EVENT: &str = "local-data://changed";

static VOCABULARY_ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const SCHEMA_SQL: &str = r#"
CREATE TABLE usage_receipts (
    session_id TEXT PRIMARY KEY NOT NULL CHECK(length(session_id) BETWEEN 1 AND 160),
    started_at_ms INTEGER NOT NULL CHECK(started_at_ms >= 0),
    completed_at_ms INTEGER NOT NULL CHECK(completed_at_ms >= started_at_ms),
    local_date TEXT NOT NULL CHECK(length(local_date) = 10),
    word_count INTEGER NOT NULL CHECK(word_count >= 0),
    speech_duration_ms INTEGER NOT NULL CHECK(speech_duration_ms >= 0),
    language_tag TEXT NOT NULL CHECK(length(language_tag) BETWEEN 1 AND 35),
    engine_id TEXT NOT NULL CHECK(length(engine_id) BETWEEN 1 AND 200),
    delivery_status TEXT NOT NULL CHECK(delivery_status IN (
        'inserted', 'focusChanged', 'secureField', 'accessibilityMissing',
        'unsupported', 'failed', 'indeterminate'
    ))
) STRICT, WITHOUT ROWID;

CREATE TABLE usage_daily (
    local_date TEXT PRIMARY KEY NOT NULL CHECK(length(local_date) = 10),
    sessions INTEGER NOT NULL CHECK(sessions >= 0),
    words INTEGER NOT NULL CHECK(words >= 0),
    speech_duration_ms INTEGER NOT NULL CHECK(speech_duration_ms >= 0)
) STRICT, WITHOUT ROWID;

CREATE TABLE usage_daily_language (
    local_date TEXT NOT NULL CHECK(length(local_date) = 10),
    language_tag TEXT NOT NULL CHECK(length(language_tag) BETWEEN 1 AND 35),
    sessions INTEGER NOT NULL CHECK(sessions >= 0),
    words INTEGER NOT NULL CHECK(words >= 0),
    speech_duration_ms INTEGER NOT NULL CHECK(speech_duration_ms >= 0),
    PRIMARY KEY (local_date, language_tag),
    FOREIGN KEY (local_date) REFERENCES usage_daily(local_date) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

CREATE TABLE transcript_history (
    session_id TEXT PRIMARY KEY NOT NULL,
    started_at_ms INTEGER NOT NULL CHECK(started_at_ms >= 0),
    completed_at_ms INTEGER NOT NULL CHECK(completed_at_ms >= started_at_ms),
    text TEXT NOT NULL CHECK(length(text) > 0),
    word_count INTEGER NOT NULL CHECK(word_count >= 0),
    speech_duration_ms INTEGER NOT NULL CHECK(speech_duration_ms >= 0),
    language_tag TEXT NOT NULL CHECK(length(language_tag) BETWEEN 1 AND 35),
    engine_id TEXT NOT NULL CHECK(length(engine_id) BETWEEN 1 AND 200),
    target_app TEXT,
    delivery_status TEXT NOT NULL CHECK(delivery_status IN (
        'inserted', 'focusChanged', 'accessibilityMissing', 'unsupported',
        'failed', 'indeterminate'
    )),
    FOREIGN KEY (session_id) REFERENCES usage_receipts(session_id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

CREATE INDEX transcript_history_page
    ON transcript_history(completed_at_ms DESC, session_id DESC);

CREATE TABLE vocabulary_entries (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 1 AND 160),
    phrase TEXT NOT NULL CHECK(length(phrase) > 0),
    spoken_form TEXT,
    category TEXT NOT NULL CHECK(category IN ('name', 'technical', 'company', 'replacement')),
    language_tag TEXT,
    enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
    created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
) STRICT, WITHOUT ROWID;

PRAGMA user_version = 1;
"#;

#[derive(Debug)]
pub struct LocalDataStore {
    inner: LocalDataStoreInner,
    prompt_cache: RwLock<Arc<[String]>>,
}

#[derive(Debug)]
enum LocalDataStoreInner {
    Available(Mutex<Connection>),
    Unavailable(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetrics {
    pub sessions: u64,
    pub words: u64,
    pub speech_duration_ms: u64,
    pub average_wpm: Option<u64>,
    pub estimated_time_saved_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDay {
    pub local_date: String,
    pub sessions: u64,
    pub words: u64,
    pub speech_duration_ms: u64,
    pub average_wpm: Option<u64>,
    pub estimated_time_saved_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLanguage {
    pub language_tag: String,
    pub sessions: u64,
    pub words: u64,
    pub speech_duration_ms: u64,
    pub average_wpm: Option<u64>,
    pub estimated_time_saved_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDashboard {
    pub generated_at_ms: u64,
    pub days_requested: u16,
    pub duration_basis: &'static str,
    pub typing_baseline_wpm: u16,
    pub today: UsageMetrics,
    pub period: UsageMetrics,
    pub previous_period: Option<UsageMetrics>,
    pub lifetime: UsageMetrics,
    pub days: Vec<UsageDay>,
    pub languages: Vec<UsageLanguage>,
    pub saved_transcript_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryCursor {
    pub completed_at_ms: u64,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptHistoryItem {
    pub session_id: String,
    pub started_at_ms: u64,
    pub completed_at_ms: u64,
    pub text: String,
    pub word_count: u64,
    pub speech_duration_ms: u64,
    pub language_tag: String,
    pub engine_id: String,
    pub target_app: Option<String>,
    pub delivery_status: DictationDeliveryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub items: Vec<TranscriptHistoryItem>,
    pub next_cursor: Option<HistoryCursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VocabularyCategory {
    Name,
    Technical,
    Company,
    Replacement,
}

impl VocabularyCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Technical => "technical",
            Self::Company => "company",
            Self::Replacement => "replacement",
        }
    }

    fn from_database(value: &str) -> Result<Self, String> {
        match value {
            "name" => Ok(Self::Name),
            "technical" => Ok(Self::Technical),
            "company" => Ok(Self::Company),
            "replacement" => Ok(Self::Replacement),
            _ => Err("the vocabulary database contains an invalid category".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyInput {
    pub phrase: String,
    pub spoken_form: Option<String>,
    pub category: VocabularyCategory,
    pub language_tag: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyEntryDto {
    pub id: String,
    pub phrase: String,
    pub spoken_form: Option<String>,
    pub category: VocabularyCategory,
    pub language_tag: Option<String>,
    pub enabled: bool,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteVocabularyResult {
    pub id: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClearLocalDataScope {
    TranscriptHistory,
    UsageAndHistory,
    Vocabulary,
    All,
}

impl ClearLocalDataScope {
    pub fn clears_usage(self) -> bool {
        matches!(self, Self::UsageAndHistory | Self::All)
    }

    pub fn clears_history(self) -> bool {
        matches!(
            self,
            Self::TranscriptHistory | Self::UsageAndHistory | Self::All
        )
    }

    pub fn clears_vocabulary(self) -> bool {
        matches!(self, Self::Vocabulary | Self::All)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearLocalDataResult {
    pub scope: ClearLocalDataScope,
    pub deleted_usage_sessions: u64,
    pub deleted_transcripts: u64,
    pub deleted_vocabulary_entries: u64,
    pub cleared_latest_transcript: bool,
    pub cleared_latest_session_id: Option<String>,
    /// False means logical deletion committed but another SQLite reader kept
    /// the WAL from being physically truncated. Retrying clear after closing
    /// the other reader finishes the privacy cleanup.
    pub storage_cleanup_complete: bool,
    pub storage_cleanup_warning: Option<String>,
    pub memory_cleanup_complete: bool,
    pub memory_cleanup_warning: Option<String>,
    pub cleared_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalDataDomain {
    Usage,
    History,
    Vocabulary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDataChangedEvent {
    pub revision: u64,
    pub domains: Vec<LocalDataDomain>,
}

#[derive(Debug, Clone)]
pub struct CompletedDictationRecord<'a> {
    pub session_id: &'a str,
    pub started_at_ms: u64,
    pub completed_at_ms: u64,
    pub text: &'a str,
    pub speech_duration_ms: u64,
    pub language_tag: &'a str,
    pub engine_id: &'a str,
    pub target_app: Option<&'a str>,
    pub delivery_status: DictationDeliveryStatus,
    pub save_history: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecordOutcome {
    pub inserted_usage: bool,
    pub inserted_history: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedVocabularyInput {
    phrase: String,
    spoken_form: Option<String>,
    category: VocabularyCategory,
    language_tag: Option<String>,
    enabled: bool,
}

impl LocalDataStore {
    pub fn open(database_path: PathBuf) -> Self {
        let (inner, prompt_cache) = match open_database(&database_path) {
            Ok(connection) => match prompt_snapshot_from_connection(&connection) {
                Ok(prompt) => (
                    LocalDataStoreInner::Available(Mutex::new(connection)),
                    Arc::from(prompt),
                ),
                Err(error) => (
                    LocalDataStoreInner::Unavailable(format!(
                        "the local vocabulary cannot be loaded: {error}"
                    )),
                    Arc::from([]),
                ),
            },
            Err(error) => (LocalDataStoreInner::Unavailable(error), Arc::from([])),
        };
        Self {
            inner,
            prompt_cache: RwLock::new(prompt_cache),
        }
    }

    pub fn unavailable_reason(&self) -> Option<&str> {
        match &self.inner {
            LocalDataStoreInner::Available(_) => None,
            LocalDataStoreInner::Unavailable(reason) => Some(reason),
        }
    }

    fn connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        match &self.inner {
            LocalDataStoreInner::Available(connection) => connection
                .lock()
                .map_err(|_| "local data storage is unavailable".into()),
            LocalDataStoreInner::Unavailable(reason) => {
                Err(format!("local data storage is unavailable: {reason}"))
            }
        }
    }

    pub fn record_completed_dictation(
        &self,
        record: CompletedDictationRecord<'_>,
    ) -> Result<RecordOutcome, String> {
        let text = record.text;
        if text.trim().is_empty() {
            return Ok(RecordOutcome::default());
        }
        let local_date = local_date_for_timestamp(record.completed_at_ms)?;
        let word_count = count_words(text);
        let started_at_ms = sql_integer(record.started_at_ms, "session start")?;
        let completed_at_ms = sql_integer(record.completed_at_ms, "session completion")?;
        let speech_duration_ms = sql_integer(record.speech_duration_ms, "speech duration")?;
        let word_count_sql = sql_integer(word_count, "word count")?;
        let language_tag = validated_record_language(record.language_tag);
        let engine_id = record.engine_id.trim();
        if engine_id.is_empty() || engine_id.len() > 200 {
            return Err("the completed dictation has an invalid engine identifier".into());
        }
        let delivery_status = delivery_status_to_str(record.delivery_status);
        let history_allowed =
            record.save_history && record.delivery_status != DictationDeliveryStatus::SecureField;

        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let inserted = transaction
            .execute(
                "INSERT INTO usage_receipts (
                    session_id, started_at_ms, completed_at_ms, local_date, word_count,
                    speech_duration_ms, language_tag, engine_id, delivery_status
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(session_id) DO NOTHING",
                params![
                    record.session_id,
                    started_at_ms,
                    completed_at_ms,
                    local_date,
                    word_count_sql,
                    speech_duration_ms,
                    language_tag,
                    engine_id,
                    delivery_status,
                ],
            )
            .map_err(database_error)?
            == 1;
        if !inserted {
            let matches = transaction
                .query_row(
                    "SELECT started_at_ms = ?2 AND completed_at_ms = ?3 AND local_date = ?4
                            AND word_count = ?5 AND speech_duration_ms = ?6
                            AND language_tag = ?7 AND engine_id = ?8 AND delivery_status = ?9
                     FROM usage_receipts WHERE session_id = ?1",
                    params![
                        record.session_id,
                        started_at_ms,
                        completed_at_ms,
                        local_date,
                        word_count_sql,
                        speech_duration_ms,
                        language_tag,
                        engine_id,
                        delivery_status,
                    ],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(database_error)?;
            transaction.commit().map_err(database_error)?;
            return if matches {
                Ok(RecordOutcome::default())
            } else {
                Err(
                    "a completed dictation collided with a different persisted session identifier"
                        .into(),
                )
            };
        }

        transaction
            .execute(
                "INSERT INTO usage_daily (local_date, sessions, words, speech_duration_ms)
                 VALUES (?1, 1, ?2, ?3)
                 ON CONFLICT(local_date) DO UPDATE SET
                    sessions = sessions + 1,
                    words = words + excluded.words,
                    speech_duration_ms = speech_duration_ms + excluded.speech_duration_ms",
                params![local_date, word_count_sql, speech_duration_ms],
            )
            .map_err(database_error)?;
        transaction
            .execute(
                "INSERT INTO usage_daily_language (
                    local_date, language_tag, sessions, words, speech_duration_ms
                 ) VALUES (?1, ?2, 1, ?3, ?4)
                 ON CONFLICT(local_date, language_tag) DO UPDATE SET
                    sessions = sessions + 1,
                    words = words + excluded.words,
                    speech_duration_ms = speech_duration_ms + excluded.speech_duration_ms",
                params![local_date, language_tag, word_count_sql, speech_duration_ms],
            )
            .map_err(database_error)?;

        let inserted_history = if history_allowed {
            transaction
                .execute(
                    "INSERT INTO transcript_history (
                        session_id, started_at_ms, completed_at_ms, text, word_count,
                        speech_duration_ms, language_tag, engine_id, target_app, delivery_status
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        record.session_id,
                        started_at_ms,
                        completed_at_ms,
                        text,
                        word_count_sql,
                        speech_duration_ms,
                        language_tag,
                        engine_id,
                        record
                            .target_app
                            .map(str::trim)
                            .filter(|value| !value.is_empty()),
                        delivery_status,
                    ],
                )
                .map_err(database_error)?
                == 1
        } else {
            false
        };
        transaction.commit().map_err(database_error)?;
        Ok(RecordOutcome {
            inserted_usage: true,
            inserted_history,
        })
    }

    pub fn usage_dashboard(&self, days: u16) -> Result<UsageDashboard, String> {
        if !(1..=366).contains(&days) {
            return Err("usage dashboard days must be between 1 and 366".into());
        }
        let generated_at_ms = now_ms();
        let today = local_date_for_timestamp(generated_at_ms)?;
        self.usage_dashboard_at(days, generated_at_ms, &today)
    }

    fn usage_dashboard_at(
        &self,
        days: u16,
        generated_at_ms: u64,
        today: &str,
    ) -> Result<UsageDashboard, String> {
        let today_date = parse_local_date(today)?;
        let period_start = today_date
            .checked_sub_days(Days::new(u64::from(days - 1)))
            .ok_or_else(|| "usage period is outside the supported date range".to_string())?;
        let previous_end = period_start.checked_sub_days(Days::new(1)).ok_or_else(|| {
            "previous usage period is outside the supported date range".to_string()
        })?;
        let previous_start = previous_end
            .checked_sub_days(Days::new(u64::from(days - 1)))
            .ok_or_else(|| {
                "previous usage period is outside the supported date range".to_string()
            })?;
        let period_start = format_local_date(period_start);
        let previous_start = format_local_date(previous_start);
        let previous_end = format_local_date(previous_end);

        let connection = self.connection()?;
        let today_metrics = query_metrics(&connection, "local_date = ?1", params![today])?;
        let period = query_metrics(
            &connection,
            "local_date BETWEEN ?1 AND ?2",
            params![period_start, today],
        )?;
        let previous = query_metrics(
            &connection,
            "local_date BETWEEN ?1 AND ?2",
            params![previous_start, previous_end],
        )?;
        let lifetime = query_metrics(&connection, "1 = 1", params![])?;

        let mut by_date = HashMap::new();
        {
            let mut statement = connection
                .prepare(
                    "SELECT local_date, sessions, words, speech_duration_ms
                     FROM usage_daily
                     WHERE local_date BETWEEN ?1 AND ?2
                     ORDER BY local_date ASC",
                )
                .map_err(database_error)?;
            let rows = statement
                .query_map(params![period_start, today], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        metrics_from_sql(row.get(1)?, row.get(2)?, row.get(3)?)?,
                    ))
                })
                .map_err(database_error)?;
            for row in rows {
                let (date, metrics) = row.map_err(database_error)?;
                by_date.insert(date, metrics);
            }
        }

        let mut day_rows = Vec::with_capacity(usize::from(days));
        let mut cursor = parse_local_date(&period_start)?;
        for _ in 0..days {
            let local_date = format_local_date(cursor);
            let metrics = by_date.remove(&local_date).unwrap_or_default();
            day_rows.push(UsageDay::from_metrics(local_date, metrics));
            cursor = cursor
                .checked_add_days(Days::new(1))
                .ok_or_else(|| "usage date is outside the supported range".to_string())?;
        }

        let mut languages = Vec::new();
        {
            let mut statement = connection
                .prepare(
                    "SELECT language_tag, SUM(sessions), SUM(words), SUM(speech_duration_ms)
                     FROM usage_daily_language
                     WHERE local_date BETWEEN ?1 AND ?2
                     GROUP BY language_tag
                     ORDER BY SUM(words) DESC, language_tag ASC",
                )
                .map_err(database_error)?;
            let rows = statement
                .query_map(params![period_start, today], |row| {
                    let language_tag = row.get::<_, String>(0)?;
                    let metrics = metrics_from_sql(row.get(1)?, row.get(2)?, row.get(3)?)?;
                    Ok(UsageLanguage::from_metrics(language_tag, metrics))
                })
                .map_err(database_error)?;
            for row in rows {
                languages.push(row.map_err(database_error)?);
            }
        }
        let saved_transcript_count = connection
            .query_row("SELECT COUNT(*) FROM transcript_history", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(database_error)
            .and_then(nonnegative_sql_integer)?;

        Ok(UsageDashboard {
            generated_at_ms,
            days_requested: days,
            duration_basis: "capture",
            typing_baseline_wpm: TYPING_BASELINE_WPM,
            today: today_metrics,
            period,
            previous_period: (previous.sessions > 0).then_some(previous),
            lifetime,
            days: day_rows,
            languages,
            saved_transcript_count,
        })
    }

    pub fn transcript_history(
        &self,
        cursor: Option<HistoryCursor>,
        limit: Option<u16>,
    ) -> Result<HistoryPage, String> {
        let limit = limit.unwrap_or(DEFAULT_HISTORY_LIMIT);
        if !(1..=MAX_HISTORY_LIMIT).contains(&limit) {
            return Err(format!(
                "transcript history limit must be between 1 and {MAX_HISTORY_LIMIT}"
            ));
        }
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.session_id.trim().is_empty())
        {
            return Err("transcript history cursor has no session identifier".into());
        }
        let fetch_limit = i64::from(limit) + 1;
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT session_id, started_at_ms, completed_at_ms, text, word_count,
                        speech_duration_ms, language_tag, engine_id, target_app, delivery_status
                 FROM transcript_history
                 WHERE (?1 IS NULL)
                    OR completed_at_ms < ?1
                    OR (completed_at_ms = ?1 AND session_id < ?2)
                 ORDER BY completed_at_ms DESC, session_id DESC
                 LIMIT ?3",
            )
            .map_err(database_error)?;
        let cursor_ms = cursor
            .as_ref()
            .map(|cursor| sql_integer(cursor.completed_at_ms, "history cursor"))
            .transpose()?;
        let cursor_id = cursor.as_ref().map(|cursor| cursor.session_id.as_str());
        let rows = statement
            .query_map(
                params![cursor_ms, cursor_id, fetch_limit],
                history_item_from_row,
            )
            .map_err(database_error)?;
        let mut items = Vec::with_capacity(usize::from(limit) + 1);
        for row in rows {
            items.push(row.map_err(database_error)?);
        }
        let has_next = items.len() > usize::from(limit);
        if has_next {
            items.pop();
        }
        let next_cursor = has_next.then(|| {
            let last = items
                .last()
                .expect("a non-empty limited page has a last item");
            HistoryCursor {
                completed_at_ms: last.completed_at_ms,
                session_id: last.session_id.clone(),
            }
        });
        Ok(HistoryPage { items, next_cursor })
    }

    pub fn list_vocabulary(&self) -> Result<Vec<VocabularyEntryDto>, String> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, phrase, spoken_form, category, language_tag, enabled,
                        created_at_ms, updated_at_ms
                 FROM vocabulary_entries
                 ORDER BY enabled DESC, lower(phrase) ASC, phrase ASC, id ASC",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map([], vocabulary_entry_from_row)
            .map_err(database_error)?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row.map_err(database_error)?);
        }
        Ok(entries)
    }

    pub fn create_vocabulary(&self, input: VocabularyInput) -> Result<VocabularyEntryDto, String> {
        let input = validate_vocabulary_input(input)?;
        let created_at_ms = now_ms();
        let created_at_sql = sql_integer(created_at_ms, "vocabulary creation time")?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let mut created_id = None;
        for _ in 0..8 {
            let sequence = VOCABULARY_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
            let id = format!("vocabulary-{created_at_ms}-{sequence}");
            let result = transaction.execute(
                "INSERT INTO vocabulary_entries (
                    id, phrase, spoken_form, category, language_tag, enabled,
                    created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![
                    id,
                    input.phrase,
                    input.spoken_form,
                    input.category.as_str(),
                    input.language_tag,
                    input.enabled,
                    created_at_sql,
                ],
            );
            match result {
                Ok(1) => {
                    created_id = Some(id);
                    break;
                }
                Err(error) if is_constraint_error(&error) => continue,
                Err(error) => return Err(database_error(error)),
                Ok(_) => return Err("the vocabulary entry was not created".into()),
            }
        }
        let id =
            created_id.ok_or_else(|| "could not allocate a vocabulary identifier".to_string())?;
        prompt_snapshot_from_connection(&transaction)?;
        transaction.commit().map_err(database_error)?;
        self.refresh_prompt_cache(&connection);
        Ok(VocabularyEntryDto {
            id,
            phrase: input.phrase,
            spoken_form: input.spoken_form,
            category: input.category,
            language_tag: input.language_tag,
            enabled: input.enabled,
            created_at_ms,
            updated_at_ms: created_at_ms,
        })
    }

    pub fn update_vocabulary(
        &self,
        id: &str,
        input: VocabularyInput,
    ) -> Result<VocabularyEntryDto, String> {
        let id = validate_identifier(id, "vocabulary identifier")?;
        let input = validate_vocabulary_input(input)?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let existing = transaction
            .query_row(
                "SELECT created_at_ms, updated_at_ms FROM vocabulary_entries WHERE id = ?1",
                [id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| "the vocabulary entry no longer exists".to_string())?;
        let created_at_ms = nonnegative_sql_integer(existing.0)?;
        let previous_updated_at_ms = nonnegative_sql_integer(existing.1)?;
        let updated_at_ms = now_ms().max(previous_updated_at_ms.saturating_add(1));
        let updated_at_sql = sql_integer(updated_at_ms, "vocabulary update time")?;
        transaction
            .execute(
                "UPDATE vocabulary_entries
                 SET phrase = ?2, spoken_form = ?3, category = ?4, language_tag = ?5,
                     enabled = ?6, updated_at_ms = ?7
                 WHERE id = ?1",
                params![
                    id,
                    input.phrase,
                    input.spoken_form,
                    input.category.as_str(),
                    input.language_tag,
                    input.enabled,
                    updated_at_sql,
                ],
            )
            .map_err(database_error)?;
        prompt_snapshot_from_connection(&transaction)?;
        transaction.commit().map_err(database_error)?;
        self.refresh_prompt_cache(&connection);
        Ok(VocabularyEntryDto {
            id: id.to_owned(),
            phrase: input.phrase,
            spoken_form: input.spoken_form,
            category: input.category,
            language_tag: input.language_tag,
            enabled: input.enabled,
            created_at_ms,
            updated_at_ms,
        })
    }

    pub fn delete_vocabulary(&self, id: &str) -> Result<DeleteVocabularyResult, String> {
        let id = validate_identifier(id, "vocabulary identifier")?;
        let connection = self.connection()?;
        let deleted = connection
            .execute("DELETE FROM vocabulary_entries WHERE id = ?1", [id])
            .map_err(database_error)?
            == 1;
        if deleted {
            self.refresh_prompt_cache(&connection);
        }
        Ok(DeleteVocabularyResult {
            id: id.to_owned(),
            deleted,
        })
    }

    /// Return a non-blocking, immutable session snapshot. If a mutation owns
    /// the cache for its tiny post-commit refresh, dictation starts with no
    /// hints instead of waiting behind optional local data work.
    pub fn vocabulary_prompt_snapshot(&self) -> Arc<[String]> {
        self.prompt_cache
            .try_read()
            .map(|prompt| Arc::clone(&prompt))
            .unwrap_or_else(|_| Arc::from([]))
    }

    pub fn clear(&self, scope: ClearLocalDataScope) -> Result<ClearLocalDataResult, String> {
        let mut connection = self.connection()?;
        let clears_private_text = scope.clears_history() || scope.clears_vocabulary();
        if clears_private_text {
            // Remove any older frames before the delete. The post-commit pass
            // below then removes the securely-deleted page images themselves.
            truncate_wal_after_private_deletion(&connection)?;
        }
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let deleted_usage_sessions = if scope.clears_usage() {
            count_table(&transaction, "usage_receipts")?
        } else {
            0
        };
        let deleted_transcripts = if scope.clears_history() {
            count_table(&transaction, "transcript_history")?
        } else {
            0
        };
        let deleted_vocabulary_entries = if scope.clears_vocabulary() {
            count_table(&transaction, "vocabulary_entries")?
        } else {
            0
        };

        if scope.clears_history() && !scope.clears_usage() {
            transaction
                .execute("DELETE FROM transcript_history", [])
                .map_err(database_error)?;
        }
        if scope.clears_usage() {
            // Delete explicitly as well as declaring cascades. A database that
            // somehow lost an FK can never make a privacy clear leave history.
            transaction
                .execute("DELETE FROM transcript_history", [])
                .map_err(database_error)?;
            transaction
                .execute("DELETE FROM usage_receipts", [])
                .map_err(database_error)?;
            transaction
                .execute("DELETE FROM usage_daily_language", [])
                .map_err(database_error)?;
            transaction
                .execute("DELETE FROM usage_daily", [])
                .map_err(database_error)?;
        }
        if scope.clears_vocabulary() {
            transaction
                .execute("DELETE FROM vocabulary_entries", [])
                .map_err(database_error)?;
        }
        transaction.commit().map_err(database_error)?;
        if scope.clears_vocabulary() {
            self.refresh_prompt_cache(&connection);
        }
        let storage_cleanup_warning = if clears_private_text {
            truncate_wal_after_private_deletion(&connection).err()
        } else {
            None
        };
        Ok(ClearLocalDataResult {
            scope,
            deleted_usage_sessions,
            deleted_transcripts,
            deleted_vocabulary_entries,
            cleared_latest_transcript: false,
            cleared_latest_session_id: None,
            storage_cleanup_complete: storage_cleanup_warning.is_none(),
            storage_cleanup_warning,
            memory_cleanup_complete: true,
            memory_cleanup_warning: None,
            cleared_at_ms: now_ms(),
        })
    }

    fn refresh_prompt_cache(&self, connection: &Connection) {
        let Ok(prompt) = prompt_snapshot_from_connection(connection) else {
            // Native mutations validate before commit. If an external writer
            // corrupts the invariant, retain the last known-complete prompt
            // rather than silently replacing every hint with an empty set.
            return;
        };
        if let Ok(mut cache) = self.prompt_cache.write() {
            *cache = Arc::from(prompt);
        }
    }
}

impl UsageMetrics {
    fn new(sessions: u64, words: u64, speech_duration_ms: u64) -> Self {
        let average_wpm = average_wpm(words, speech_duration_ms);
        let typing_duration_ms = words
            .saturating_mul(60_000)
            .checked_div(u64::from(TYPING_BASELINE_WPM))
            .unwrap_or(0);
        Self {
            sessions,
            words,
            speech_duration_ms,
            average_wpm,
            estimated_time_saved_ms: typing_duration_ms.saturating_sub(speech_duration_ms),
        }
    }
}

impl UsageDay {
    fn from_metrics(local_date: String, metrics: UsageMetrics) -> Self {
        Self {
            local_date,
            sessions: metrics.sessions,
            words: metrics.words,
            speech_duration_ms: metrics.speech_duration_ms,
            average_wpm: metrics.average_wpm,
            estimated_time_saved_ms: metrics.estimated_time_saved_ms,
        }
    }
}

impl UsageLanguage {
    fn from_metrics(language_tag: String, metrics: UsageMetrics) -> Self {
        Self {
            language_tag,
            sessions: metrics.sessions,
            words: metrics.words,
            speech_duration_ms: metrics.speech_duration_ms,
            average_wpm: metrics.average_wpm,
            estimated_time_saved_ms: metrics.estimated_time_saved_ms,
        }
    }
}

fn open_database(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create local data directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let existed = path.exists();
    let flags = if existed {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
    };
    let mut connection = Connection::open_with_flags(path, flags)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    if version > SCHEMA_VERSION {
        return Err(format!(
            "{} uses unsupported local data schema version {version}; it was left unchanged",
            path.display()
        ));
    }
    if version < 0 {
        return Err(format!(
            "{} has an invalid local data schema version; it was left unchanged",
            path.display()
        ));
    }
    if existed {
        verify_integrity(&connection, path)?;
        if version == 0 && database_has_user_objects(&connection)? {
            return Err(format!(
                "{} is not an empty Spick database; it was left unchanged",
                path.display()
            ));
        }
        if version == SCHEMA_VERSION {
            validate_v1_schema(&connection, path)?;
        }
    }

    configure_connection(&connection, path)?;
    if version == 0 {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        transaction
            .execute_batch(SCHEMA_SQL)
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)?;
    }
    Ok(connection)
}

fn configure_connection(connection: &Connection, path: &Path) -> Result<(), String> {
    connection
        .busy_timeout(DATABASE_BUSY_TIMEOUT)
        .map_err(database_error)?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(database_error)?;
    connection
        .pragma_update(None, "secure_delete", "ON")
        .map_err(database_error)?;
    let journal_mode = connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })
        .map_err(database_error)?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(format!(
            "{} could not enable the required WAL journal mode",
            path.display()
        ));
    }
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(database_error)?;
    Ok(())
}

fn verify_integrity(connection: &Connection, path: &Path) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA quick_check(1)")
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    let mut rows = statement
        .query([])
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    let result = rows
        .next()
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?
        .ok_or_else(|| format!("{} returned no integrity result", path.display()))?
        .get::<_, String>(0)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    if result != "ok" {
        return Err(format!(
            "{} failed its integrity check and was left unchanged",
            path.display()
        ));
    }
    let foreign_key_problem = connection
        .query_row("SELECT 1 FROM pragma_foreign_key_check LIMIT 1", [], |_| {
            Ok(())
        })
        .optional()
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    if foreign_key_problem.is_some() {
        return Err(format!(
            "{} failed its foreign-key integrity check and was left unchanged",
            path.display()
        ));
    }
    Ok(())
}

fn database_has_user_objects(connection: &Connection) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_schema
                WHERE name NOT LIKE 'sqlite_%'
             )",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)
}

fn validate_v1_schema(connection: &Connection, path: &Path) -> Result<(), String> {
    let mut objects = connection
        .prepare(
            "SELECT type, name FROM sqlite_schema
             WHERE name NOT LIKE 'sqlite_%'
             ORDER BY type, name",
        )
        .map_err(|error| invalid_schema_error(path, error))?;
    let actual_objects = objects
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| invalid_schema_error(path, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| invalid_schema_error(path, error))?;
    let expected_objects = vec![
        ("index".to_string(), "transcript_history_page".to_string()),
        ("table".to_string(), "transcript_history".to_string()),
        ("table".to_string(), "usage_daily".to_string()),
        ("table".to_string(), "usage_daily_language".to_string()),
        ("table".to_string(), "usage_receipts".to_string()),
        ("table".to_string(), "vocabulary_entries".to_string()),
    ];
    if actual_objects != expected_objects {
        return Err(invalid_schema_error(
            path,
            "unexpected or missing database objects",
        ));
    }

    const CHECKS: [&str; 5] = [
        "SELECT session_id, started_at_ms, completed_at_ms, local_date, word_count, speech_duration_ms, language_tag, engine_id, delivery_status FROM usage_receipts LIMIT 0",
        "SELECT local_date, sessions, words, speech_duration_ms FROM usage_daily LIMIT 0",
        "SELECT local_date, language_tag, sessions, words, speech_duration_ms FROM usage_daily_language LIMIT 0",
        "SELECT session_id, started_at_ms, completed_at_ms, text, word_count, speech_duration_ms, language_tag, engine_id, target_app, delivery_status FROM transcript_history LIMIT 0",
        "SELECT id, phrase, spoken_form, category, language_tag, enabled, created_at_ms, updated_at_ms FROM vocabulary_entries LIMIT 0",
    ];
    for check in CHECKS {
        connection.prepare(check).map_err(|error| {
            format!(
                "{} has an invalid local data schema and was left unchanged: {error}",
                path.display()
            )
        })?;
    }
    const TABLE_REQUIREMENTS: [(&str, &[&str]); 5] = [
        (
            "usage_receipts",
            &["STRICT", "WITHOUTROWID", "CHECK(DELIVERY_STATUSIN("],
        ),
        (
            "usage_daily",
            &["STRICT", "WITHOUTROWID", "CHECK(SESSIONS>=0)"],
        ),
        (
            "usage_daily_language",
            &["STRICT", "WITHOUTROWID", "ONDELETECASCADE"],
        ),
        (
            "transcript_history",
            &["STRICT", "WITHOUTROWID", "ONDELETECASCADE"],
        ),
        (
            "vocabulary_entries",
            &["STRICT", "WITHOUTROWID", "CHECK(ENABLEDIN(0,1))"],
        ),
    ];
    for (table, requirements) in TABLE_REQUIREMENTS {
        let sql = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| invalid_schema_error(path, error))?;
        let normalized = normalize_schema_sql(&sql);
        if requirements
            .iter()
            .any(|requirement| !normalized.contains(requirement))
        {
            return Err(invalid_schema_error(
                path,
                "required table guarantees are missing",
            ));
        }
    }
    validate_foreign_key(
        connection,
        path,
        "usage_daily_language",
        "usage_daily",
        "local_date",
        "local_date",
    )?;
    validate_foreign_key(
        connection,
        path,
        "transcript_history",
        "usage_receipts",
        "session_id",
        "session_id",
    )?;
    let index_sql = connection
        .query_row(
            "SELECT sql FROM sqlite_schema
             WHERE type = 'index' AND name = 'transcript_history_page'",
            [],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| invalid_schema_error(path, error))?;
    if !normalize_schema_sql(&index_sql)
        .contains("ONTRANSCRIPT_HISTORY(COMPLETED_AT_MSDESC,SESSION_IDDESC)")
    {
        return Err(invalid_schema_error(
            path,
            "the history paging index is invalid",
        ));
    }
    Ok(())
}

fn validate_foreign_key(
    connection: &Connection,
    path: &Path,
    table: &str,
    referenced_table: &str,
    from_column: &str,
    to_column: &str,
) -> Result<(), String> {
    let pragma = match table {
        "usage_daily_language" => "PRAGMA foreign_key_list(usage_daily_language)",
        "transcript_history" => "PRAGMA foreign_key_list(transcript_history)",
        _ => return Err(invalid_schema_error(path, "unexpected foreign-key table")),
    };
    let mut statement = connection
        .prepare(pragma)
        .map_err(|error| invalid_schema_error(path, error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(|error| invalid_schema_error(path, error))?;
    let mut matches = false;
    for row in rows {
        let (target, from, to, on_delete) =
            row.map_err(|error| invalid_schema_error(path, error))?;
        matches |= target == referenced_table
            && from == from_column
            && to == to_column
            && on_delete.eq_ignore_ascii_case("cascade");
    }
    if !matches {
        return Err(invalid_schema_error(
            path,
            "a required foreign key is missing",
        ));
    }
    Ok(())
}

fn normalize_schema_sql(sql: &str) -> String {
    sql.chars()
        .filter(|character| !character.is_whitespace() && *character != '"' && *character != '`')
        .flat_map(char::to_uppercase)
        .collect()
}

fn invalid_schema_error(path: &Path, reason: impl std::fmt::Display) -> String {
    format!(
        "{} has an invalid local data schema and was left unchanged: {reason}",
        path.display()
    )
}

fn query_metrics<P: rusqlite::Params>(
    connection: &Connection,
    predicate: &str,
    parameters: P,
) -> Result<UsageMetrics, String> {
    let sql = format!(
        "SELECT COALESCE(SUM(sessions), 0), COALESCE(SUM(words), 0),
                COALESCE(SUM(speech_duration_ms), 0)
         FROM usage_daily WHERE {predicate}"
    );
    connection
        .query_row(&sql, parameters, |row| {
            metrics_from_sql(row.get(0)?, row.get(1)?, row.get(2)?)
        })
        .map_err(database_error)
}

fn prompt_snapshot_from_connection(connection: &Connection) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT phrase FROM vocabulary_entries
             WHERE enabled = 1
             ORDER BY lower(phrase) ASC, phrase ASC, id ASC",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(database_error)?;
    let mut phrases = Vec::new();
    let mut used_bytes = 0usize;
    for row in rows {
        let phrase = row.map_err(database_error)?;
        let separator_bytes = usize::from(!phrases.is_empty()) * 2;
        let next_bytes = separator_bytes.saturating_add(phrase.len());
        if phrases.len() >= MAX_VOCABULARY_ENTRIES {
            return Err(format!(
                "at most {MAX_VOCABULARY_ENTRIES} vocabulary phrases can be enabled"
            ));
        }
        if used_bytes.saturating_add(next_bytes) > MAX_VOCABULARY_PROMPT_BYTES {
            return Err(format!(
                "enabled vocabulary phrases cannot exceed a {MAX_VOCABULARY_PROMPT_BYTES}-byte Whisper prompt"
            ));
        }
        used_bytes += next_bytes;
        phrases.push(phrase);
    }
    Ok(phrases)
}

fn truncate_wal_after_private_deletion(connection: &Connection) -> Result<(), String> {
    let (busy, _remaining_frames, _checkpointed_frames) = connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(database_error)?;
    if busy != 0 {
        return Err(
            "local rows were deleted, but SQLite could not yet remove old private-text frames from the WAL; close other Spick processes and retry the same clear"
                .into(),
        );
    }
    Ok(())
}

fn metrics_from_sql(
    sessions: i64,
    words: i64,
    speech_duration_ms: i64,
) -> rusqlite::Result<UsageMetrics> {
    Ok(UsageMetrics::new(
        sql_u64_for_row(sessions)?,
        sql_u64_for_row(words)?,
        sql_u64_for_row(speech_duration_ms)?,
    ))
}

fn history_item_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TranscriptHistoryItem> {
    let delivery_status = row.get::<_, String>(9)?;
    Ok(TranscriptHistoryItem {
        session_id: row.get(0)?,
        started_at_ms: sql_u64_for_row(row.get(1)?)?,
        completed_at_ms: sql_u64_for_row(row.get(2)?)?,
        text: row.get(3)?,
        word_count: sql_u64_for_row(row.get(4)?)?,
        speech_duration_ms: sql_u64_for_row(row.get(5)?)?,
        language_tag: row.get(6)?,
        engine_id: row.get(7)?,
        target_app: row.get(8)?,
        delivery_status: delivery_status_from_str(&delivery_status).map_err(|reason| {
            SqlError::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, reason)),
            )
        })?,
    })
}

fn vocabulary_entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VocabularyEntryDto> {
    let category = row.get::<_, String>(3)?;
    Ok(VocabularyEntryDto {
        id: row.get(0)?,
        phrase: row.get(1)?,
        spoken_form: row.get(2)?,
        category: VocabularyCategory::from_database(&category).map_err(|reason| {
            SqlError::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, reason)),
            )
        })?,
        language_tag: row.get(4)?,
        enabled: row.get(5)?,
        created_at_ms: sql_u64_for_row(row.get(6)?)?,
        updated_at_ms: sql_u64_for_row(row.get(7)?)?,
    })
}

fn validate_vocabulary_input(input: VocabularyInput) -> Result<ValidatedVocabularyInput, String> {
    let phrase = input.phrase.trim().to_owned();
    if phrase.is_empty() {
        return Err("a vocabulary phrase cannot be empty".into());
    }
    if phrase.contains('\0') {
        return Err("a vocabulary phrase cannot contain a null byte".into());
    }
    if phrase.len() > MAX_PHRASE_BYTES {
        return Err(format!(
            "a vocabulary phrase cannot exceed {MAX_PHRASE_BYTES} UTF-8 bytes"
        ));
    }
    if UnicodeSegmentation::graphemes(phrase.as_str(), true).count() > MAX_PHRASE_GRAPHEMES {
        return Err(format!(
            "a vocabulary phrase cannot exceed {MAX_PHRASE_GRAPHEMES} characters"
        ));
    }
    let spoken_form = input
        .spoken_form
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if spoken_form.as_ref().is_some_and(|value| {
        value.contains('\0')
            || value.len() > MAX_PHRASE_BYTES
            || UnicodeSegmentation::graphemes(value.as_str(), true).count() > MAX_PHRASE_GRAPHEMES
    }) {
        return Err(format!(
            "a spoken form cannot contain null bytes, exceed {MAX_PHRASE_GRAPHEMES} characters, or exceed {MAX_PHRASE_BYTES} UTF-8 bytes"
        ));
    }
    let language_tag = input
        .language_tag
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if let Some(language_tag) = &language_tag {
        validate_language_tag(language_tag)?;
    }
    Ok(ValidatedVocabularyInput {
        phrase,
        spoken_form,
        category: input.category,
        language_tag,
        enabled: input.enabled,
    })
}

fn validate_language_tag(language_tag: &str) -> Result<(), String> {
    if language_tag.len() > MAX_LANGUAGE_TAG_BYTES
        || language_tag.starts_with('-')
        || language_tag.ends_with('-')
        || !language_tag
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("the vocabulary language tag is invalid".into());
    }
    Ok(())
}

fn validated_record_language(language_tag: &str) -> &str {
    let language_tag = language_tag.trim();
    if language_tag.is_empty()
        || language_tag.len() > MAX_LANGUAGE_TAG_BYTES
        || language_tag.starts_with('-')
        || language_tag.ends_with('-')
        || !language_tag
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        "und"
    } else {
        language_tag
    }
}

fn validate_identifier<'a>(value: &'a str, name: &str) -> Result<&'a str, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 160 || value.contains('\0') {
        return Err(format!("the {name} is invalid"));
    }
    Ok(value)
}

fn count_words(text: &str) -> u64 {
    u64::try_from(UnicodeSegmentation::unicode_words(text).count()).unwrap_or(u64::MAX)
}

fn average_wpm(words: u64, speech_duration_ms: u64) -> Option<u64> {
    if words == 0 || speech_duration_ms == 0 {
        return None;
    }
    let numerator = u128::from(words).saturating_mul(60_000);
    let rounded = numerator.saturating_add(u128::from(speech_duration_ms / 2))
        / u128::from(speech_duration_ms);
    Some(u64::try_from(rounded).unwrap_or(u64::MAX))
}

fn count_table(connection: &Connection, table: &str) -> Result<u64, String> {
    let sql = match table {
        "usage_receipts" => "SELECT COUNT(*) FROM usage_receipts",
        "transcript_history" => "SELECT COUNT(*) FROM transcript_history",
        "vocabulary_entries" => "SELECT COUNT(*) FROM vocabulary_entries",
        _ => return Err("invalid local data table".into()),
    };
    connection
        .query_row(sql, [], |row| row.get::<_, i64>(0))
        .map_err(database_error)
        .and_then(nonnegative_sql_integer)
}

fn local_date_for_timestamp(timestamp_ms: u64) -> Result<String, String> {
    let timestamp_ms = i64::try_from(timestamp_ms)
        .map_err(|_| "the dictation timestamp is outside the supported range".to_string())?;
    let date = match Local.timestamp_millis_opt(timestamp_ms) {
        LocalResult::Single(value) => value.date_naive(),
        LocalResult::Ambiguous(first, _) => first.date_naive(),
        LocalResult::None => {
            return Err("the dictation timestamp is outside the local calendar".into())
        }
    };
    Ok(format_local_date(date))
}

fn parse_local_date(value: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| "the local usage date is invalid".into())
}

fn format_local_date(value: NaiveDate) -> String {
    value.format("%Y-%m-%d").to_string()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn sql_integer(value: u64, name: &str) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("the {name} is outside the supported range"))
}

fn nonnegative_sql_integer(value: i64) -> Result<u64, String> {
    u64::try_from(value).map_err(|_| "the local data database contains a negative value".into())
}

fn sql_u64_for_row(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|error| {
        SqlError::FromSqlConversionFailure(0, rusqlite::types::Type::Integer, Box::new(error))
    })
}

fn delivery_status_to_str(status: DictationDeliveryStatus) -> &'static str {
    match status {
        DictationDeliveryStatus::Inserted => "inserted",
        DictationDeliveryStatus::FocusChanged => "focusChanged",
        DictationDeliveryStatus::SecureField => "secureField",
        DictationDeliveryStatus::AccessibilityMissing => "accessibilityMissing",
        DictationDeliveryStatus::Unsupported => "unsupported",
        DictationDeliveryStatus::Failed => "failed",
        DictationDeliveryStatus::Indeterminate => "indeterminate",
    }
}

fn delivery_status_from_str(value: &str) -> Result<DictationDeliveryStatus, String> {
    match value {
        "inserted" => Ok(DictationDeliveryStatus::Inserted),
        "focusChanged" => Ok(DictationDeliveryStatus::FocusChanged),
        "secureField" => Ok(DictationDeliveryStatus::SecureField),
        "accessibilityMissing" => Ok(DictationDeliveryStatus::AccessibilityMissing),
        "unsupported" => Ok(DictationDeliveryStatus::Unsupported),
        "failed" => Ok(DictationDeliveryStatus::Failed),
        "indeterminate" => Ok(DictationDeliveryStatus::Indeterminate),
        _ => Err("the history database contains an invalid delivery status".into()),
    }
}

fn is_constraint_error(error: &SqlError) -> bool {
    matches!(
        error,
        SqlError::SqliteFailure(details, _)
            if details.code == ErrorCode::ConstraintViolation
    )
}

fn database_error(error: impl std::fmt::Display) -> String {
    format!("local data storage failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, LocalDataStore) {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalDataStore::open(directory.path().join("spick.sqlite3"));
        assert_eq!(store.unavailable_reason(), None);
        (directory, store)
    }

    fn record<'a>(session_id: &'a str, text: &'a str) -> CompletedDictationRecord<'a> {
        CompletedDictationRecord {
            session_id,
            started_at_ms: 1_700_000_000_000,
            completed_at_ms: 1_700_000_005_000,
            text,
            speech_duration_ms: 5_000,
            language_tag: "en",
            engine_id: "whisper-test",
            target_app: Some("Notes"),
            delivery_status: DictationDeliveryStatus::Inserted,
            save_history: false,
        }
    }

    fn vocabulary(phrase: &str) -> VocabularyInput {
        VocabularyInput {
            phrase: phrase.into(),
            spoken_form: None,
            category: VocabularyCategory::Technical,
            language_tag: Some("en".into()),
            enabled: true,
        }
    }

    #[test]
    fn migration_creates_strict_without_rowid_schema_and_pragmas() {
        let (_directory, store) = store();
        let connection = store.connection().unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            SCHEMA_VERSION
        );
        assert!(connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, bool>(0))
            .unwrap());
        assert!(connection
            .query_row("PRAGMA secure_delete", [], |row| row.get::<_, bool>(0))
            .unwrap());
        assert_eq!(
            connection
                .query_row("PRAGMA synchronous", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        let schemas: Vec<String> = {
            let mut statement = connection
                .prepare("SELECT sql FROM sqlite_schema WHERE type = 'table' ORDER BY name")
                .unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        assert_eq!(schemas.len(), 5);
        assert!(schemas
            .iter()
            .all(|sql| sql.contains("STRICT") && sql.contains("WITHOUT ROWID")));
    }

    #[test]
    fn newer_and_corrupt_databases_are_not_rewritten() {
        let directory = tempfile::tempdir().unwrap();
        let newer_path = directory.path().join("newer.sqlite3");
        {
            let connection = Connection::open(&newer_path).unwrap();
            connection.pragma_update(None, "user_version", 99).unwrap();
        }
        let newer_before = fs::read(&newer_path).unwrap();
        let newer = LocalDataStore::open(newer_path.clone());
        assert!(newer.unavailable_reason().unwrap().contains("unsupported"));
        assert_eq!(fs::read(&newer_path).unwrap(), newer_before);

        let corrupt_path = directory.path().join("corrupt.sqlite3");
        fs::write(&corrupt_path, b"not a sqlite database").unwrap();
        let corrupt_before = fs::read(&corrupt_path).unwrap();
        let corrupt = LocalDataStore::open(corrupt_path.clone());
        assert!(corrupt.unavailable_reason().is_some());
        assert_eq!(fs::read(corrupt_path).unwrap(), corrupt_before);
    }

    #[test]
    fn a_malformed_v1_schema_is_rejected_without_migration_writes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("malformed.sqlite3");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE usage_receipts (session_id TEXT PRIMARY KEY) STRICT;
                     PRAGMA user_version = 1;",
                )
                .unwrap();
        }
        let before = fs::read(&path).unwrap();

        let store = LocalDataStore::open(path.clone());

        assert!(store.unavailable_reason().unwrap().contains("invalid"));
        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[test]
    fn usage_is_idempotent_and_history_is_opt_in() {
        let (_directory, store) = store();
        let first = store
            .record_completed_dictation(record("one", "hello brave world"))
            .unwrap();
        assert_eq!(
            first,
            RecordOutcome {
                inserted_usage: true,
                inserted_history: false
            }
        );
        let mut retry = record("one", "hello brave world");
        retry.save_history = true;
        assert_eq!(
            store.record_completed_dictation(retry).unwrap(),
            RecordOutcome::default()
        );

        let today = local_date_for_timestamp(1_700_000_005_000).unwrap();
        let dashboard = store
            .usage_dashboard_at(1, 1_700_000_005_000, &today)
            .unwrap();
        assert_eq!(dashboard.today.sessions, 1);
        assert_eq!(dashboard.today.words, 3);
        assert_eq!(dashboard.saved_transcript_count, 0);

        let mut saved = record("two", "नमस्ते दुनिया");
        saved.save_history = true;
        saved.language_tag = "hi";
        assert!(
            store
                .record_completed_dictation(saved)
                .unwrap()
                .inserted_history
        );
        let history = store.transcript_history(None, None).unwrap();
        assert_eq!(history.items.len(), 1);
        assert_eq!(history.items[0].text, "नमस्ते दुनिया");
    }

    #[test]
    fn a_reused_session_id_with_different_receipt_data_is_not_a_replay() {
        let (_directory, store) = store();
        store
            .record_completed_dictation(record("one", "hello brave world"))
            .unwrap();
        let collision = record("one", "different word count entirely");
        let error = store.record_completed_dictation(collision).unwrap_err();
        assert!(error.contains("collided"));
    }

    #[test]
    fn secure_dictations_count_usage_but_never_save_text() {
        let (_directory, store) = store();
        let mut secure = record("secure", "do not persist this");
        secure.save_history = true;
        secure.delivery_status = DictationDeliveryStatus::SecureField;
        let outcome = store.record_completed_dictation(secure).unwrap();
        assert!(outcome.inserted_usage);
        assert!(!outcome.inserted_history);
        assert!(store
            .transcript_history(None, None)
            .unwrap()
            .items
            .is_empty());
    }

    #[test]
    fn dashboard_uses_unicode_words_capture_time_and_grouping() {
        let (_directory, store) = store();
        let mut english = record("english", "One two three four five");
        english.speech_duration_ms = 2_000;
        store.record_completed_dictation(english).unwrap();
        let mut hindi = record("hindi", "नमस्ते दुनिया");
        hindi.language_tag = "hi";
        hindi.speech_duration_ms = 3_000;
        store.record_completed_dictation(hindi).unwrap();

        let today = local_date_for_timestamp(1_700_000_005_000).unwrap();
        let dashboard = store
            .usage_dashboard_at(2, 1_700_000_005_000, &today)
            .unwrap();
        assert_eq!(dashboard.period.sessions, 2);
        assert_eq!(dashboard.period.words, 7);
        assert_eq!(dashboard.period.speech_duration_ms, 5_000);
        assert_eq!(dashboard.period.average_wpm, Some(84));
        assert_eq!(dashboard.days.len(), 2);
        assert_eq!(dashboard.days[0].sessions, 0);
        assert_eq!(dashboard.languages.len(), 2);
        assert_eq!(dashboard.languages[0].language_tag, "en");
        assert_eq!(dashboard.duration_basis, "capture");
        assert_eq!(dashboard.typing_baseline_wpm, 40);
    }

    #[test]
    fn history_paginates_without_duplicates() {
        let (_directory, store) = store();
        for index in 0..5 {
            let id = format!("session-{index}");
            let mut item = record(&id, "saved words");
            item.save_history = true;
            item.completed_at_ms += index;
            store.record_completed_dictation(item).unwrap();
        }
        let first = store.transcript_history(None, Some(2)).unwrap();
        assert_eq!(first.items.len(), 2);
        assert!(first.next_cursor.is_some());
        let second = store
            .transcript_history(first.next_cursor.clone(), Some(2))
            .unwrap();
        assert_eq!(second.items.len(), 2);
        assert!(first.items.iter().all(|item| second
            .items
            .iter()
            .all(|next| next.session_id != item.session_id)));
        let third = store
            .transcript_history(second.next_cursor.clone(), Some(2))
            .unwrap();
        assert_eq!(third.items.len(), 1);
        assert_eq!(third.next_cursor, None);
    }

    #[test]
    fn vocabulary_crud_validates_and_snapshot_is_deterministic() {
        let (_directory, store) = store();
        let zeta = store.create_vocabulary(vocabulary("Zeta")).unwrap();
        let alpha = store.create_vocabulary(vocabulary("alpha")).unwrap();
        let mut disabled = vocabulary("Beta");
        disabled.enabled = false;
        store.create_vocabulary(disabled).unwrap();
        let first_session_snapshot = store.vocabulary_prompt_snapshot();
        assert_eq!(
            first_session_snapshot.as_ref(),
            ["alpha".to_string(), "Zeta".to_string()]
        );
        let mut updated = vocabulary("Spick");
        updated.spoken_form = Some("speak".into());
        let updated = store.update_vocabulary(&zeta.id, updated).unwrap();
        assert_eq!(updated.phrase, "Spick");
        assert!(updated.updated_at_ms > updated.created_at_ms);
        assert_eq!(
            first_session_snapshot.as_ref(),
            ["alpha".to_string(), "Zeta".to_string()]
        );
        assert_eq!(
            store.vocabulary_prompt_snapshot().as_ref(),
            ["alpha".to_string(), "Spick".to_string()]
        );
        assert!(store.delete_vocabulary(&alpha.id).unwrap().deleted);
        assert!(!store.delete_vocabulary(&alpha.id).unwrap().deleted);
        assert!(store.create_vocabulary(vocabulary("   ")).is_err());
        let mut invalid = vocabulary("term");
        invalid.language_tag = Some("en_XX".into());
        assert!(store.create_vocabulary(invalid).is_err());
    }

    #[test]
    fn enabled_vocabulary_cannot_silently_exceed_prompt_capacity() {
        let (directory, primary_store) = store();
        for index in 0..MAX_VOCABULARY_ENTRIES {
            primary_store
                .create_vocabulary(vocabulary(&format!("term-{index}")))
                .unwrap();
        }
        assert!(primary_store
            .create_vocabulary(vocabulary("one-too-many"))
            .unwrap_err()
            .contains("at most"));
        let mut disabled = vocabulary("saved but disabled");
        disabled.enabled = false;
        let disabled = primary_store.create_vocabulary(disabled).unwrap();
        assert_eq!(primary_store.list_vocabulary().unwrap().len(), 65);
        assert!(primary_store
            .update_vocabulary(&disabled.id, vocabulary("now enabled"))
            .unwrap_err()
            .contains("at most"));
        drop(primary_store);
        {
            let connection = Connection::open(directory.path().join("spick.sqlite3")).unwrap();
            connection
                .execute(
                    "UPDATE vocabulary_entries SET enabled = 1 WHERE id = ?1",
                    [&disabled.id],
                )
                .unwrap();
        }
        let reopened = LocalDataStore::open(directory.path().join("spick.sqlite3"));
        assert!(reopened.unavailable_reason().unwrap().contains("at most"));

        let (_directory, byte_store) = store();
        for index in 0..8 {
            let phrase = format!("{}{index}", "x".repeat(118));
            byte_store.create_vocabulary(vocabulary(&phrase)).unwrap();
        }
        assert!(byte_store
            .create_vocabulary(vocabulary(&format!("{}9", "x".repeat(118))))
            .unwrap_err()
            .contains("1024-byte"));

        let huge_single_grapheme = format!("a{}", "\u{0301}".repeat(300));
        assert!(byte_store
            .create_vocabulary(vocabulary(&huge_single_grapheme))
            .unwrap_err()
            .contains("UTF-8 bytes"));
    }

    #[test]
    fn clear_scopes_report_counts_and_preserve_unselected_domains() {
        let (directory, store) = store();
        let mut saved = record("one", "saved text");
        saved.save_history = true;
        store.record_completed_dictation(saved).unwrap();
        store.create_vocabulary(vocabulary("Spick")).unwrap();

        let history = store.clear(ClearLocalDataScope::TranscriptHistory).unwrap();
        assert_eq!(history.deleted_transcripts, 1);
        assert_eq!(history.deleted_usage_sessions, 0);
        let wal_path = directory.path().join("spick.sqlite3-wal");
        assert_eq!(
            fs::metadata(wal_path)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
            0
        );
        assert_eq!(store.list_vocabulary().unwrap().len(), 1);
        let all = store.clear(ClearLocalDataScope::All).unwrap();
        assert_eq!(all.deleted_usage_sessions, 1);
        assert_eq!(all.deleted_vocabulary_entries, 1);
        let today = local_date_for_timestamp(1_700_000_005_000).unwrap();
        assert_eq!(
            store
                .usage_dashboard_at(1, 1_700_000_005_000, &today)
                .unwrap()
                .lifetime
                .sessions,
            0
        );
    }

    #[test]
    fn vocabulary_clear_also_truncates_private_wal_text() {
        let (directory, store) = store();
        store
            .create_vocabulary(vocabulary("Private project codename"))
            .unwrap();

        let result = store.clear(ClearLocalDataScope::Vocabulary).unwrap();

        assert_eq!(result.deleted_vocabulary_entries, 1);
        assert!(result.storage_cleanup_complete);
        assert_eq!(result.storage_cleanup_warning, None);
        let wal_path = directory.path().join("spick.sqlite3-wal");
        assert_eq!(
            fs::metadata(wal_path)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
            0
        );
    }

    #[test]
    fn local_change_event_serializes_metadata_only() {
        let value = serde_json::to_value(LocalDataChangedEvent {
            revision: 7,
            domains: vec![LocalDataDomain::Usage, LocalDataDomain::History],
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "revision": 7,
                "domains": ["usage", "history"]
            })
        );
        assert!(!value.to_string().contains("text"));
    }

    #[test]
    fn an_unavailable_store_isolated_from_the_dictation_result() {
        let directory = tempfile::tempdir().unwrap();
        let blocker = directory.path().join("not-a-directory");
        fs::write(&blocker, b"file").unwrap();
        let store = LocalDataStore::open(blocker.join("spick.sqlite3"));
        assert!(store.unavailable_reason().is_some());
        assert!(store
            .record_completed_dictation(record("one", "still delivered"))
            .is_err());
    }
}
