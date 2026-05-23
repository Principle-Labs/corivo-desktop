//! Single source of truth for DB-boundary time handling.
//!
//! # Why this module exists
//!
//! Earlier iterations of the repo had every caller handle timestamps itself:
//! some routes bound a `chrono::Utc::now().to_rfc3339()` string, others relied
//! on SQLite's `datetime('now')` default. The two formats are not compatible
//! (`datetime('now')` returns `YYYY-MM-DD HH:MM:SS` — space separator, no
//! timezone — which `chrono::DateTime::parse_from_rfc3339` rejects with
//! *"premature end of input"*). Any missed callsite corrupts a column and
//! blows up on read, sometimes days later.
//!
//! The fix is to turn "DB timestamp" into a type with a single write format
//! and a tolerant read parser. Repos no longer touch strings — they bind
//! `DbInstant` values directly.
//!
//! # Invariants
//!
//! * On write: canonical RFC3339 with millisecond precision and a `Z` suffix
//!   (`YYYY-MM-DDTHH:MM:SS.sssZ`). This exactly matches the schema default
//!   `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')`, so DB-generated and
//!   app-generated timestamps round-trip identically.
//! * On read: accepts any RFC3339 string first, then falls back to the legacy
//!   SQLite-native shape for rows written before this module existed. A
//!   fallback hit emits a `tracing::warn!` so we can spot lingering bad data
//!   without crashing the read.
//! * `DbInstant` *is* the only type that implements `ToSql` / `FromSql` for
//!   timestamps in the repos. `DateTime<Utc>` does not cross the DB boundary
//!   directly.
//!
//! # SQL-side rules
//!
//! * `datetime('now')` is banned in application SQL. A test at
//!   `src-tauri/tests/time_discipline.rs` fails CI if it reappears.
//! * Schema defaults use [`SQL_NOW`]. All other timestamps are bound from Rust
//!   via `DbInstant` parameters so that a test clock can actually take over.
//!
//! # Clock injection
//!
//! Production code calls [`DbInstant::now`], which delegates to a global
//! [`Clock`]. The default implementation reads the system clock. Tests swap
//! in a [`FakeClock`] via [`set_clock`] to get deterministic behaviour for
//! cooldown / "last-touched" style queries.

use std::sync::{Arc, OnceLock, RwLock};

use chrono::{DateTime, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use rusqlite::{
    types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef},
    Result as SqlResult,
};
use serde::{Deserialize, Serialize};

/// The only SQL fragment allowed to produce "now" on the DB side.
///
/// Matches [`DbInstant`]'s write format byte-for-byte so a row with a default
/// timestamp and a row with an explicitly bound `DbInstant::now()` are
/// indistinguishable.
///
/// Schema defaults are the *only* legitimate use: every INSERT/UPDATE in
/// application code should bind a `DbInstant` parameter instead, so that
/// [`set_clock`] can override time during tests. See `time_discipline`
/// integration test.
pub const SQL_NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

/// Abstract time source. Services and repos get `now` through a `Clock`
/// rather than calling `chrono::Utc::now()` directly, so tests can run
/// cooldown / decay / narrator logic without `tokio::time::sleep`.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
}

/// Production clock — just reads the system clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Test clock. Mutating `set` moves the visible "now" forward (or back); all
/// handles sharing the same `FakeClock` see the new value immediately.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct FakeClock {
    inner: Arc<RwLock<DateTime<Utc>>>,
}

impl FakeClock {
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(start)),
        }
    }

    pub fn set(&self, new_now: DateTime<Utc>) {
        *self.inner.write().expect("FakeClock poisoned") = new_now;
    }

    pub fn advance(&self, delta: chrono::Duration) {
        let mut guard = self.inner.write().expect("FakeClock poisoned");
        *guard += delta;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self.inner.read().expect("FakeClock poisoned")
    }
}

/// Global clock slot. Unset in production → `SystemClock` is used by default.
/// Tests install a `FakeClock` via [`set_clock`]; remove with [`reset_clock`].
static CLOCK: OnceLock<RwLock<Arc<dyn Clock>>> = OnceLock::new();

fn clock_slot() -> &'static RwLock<Arc<dyn Clock>> {
    CLOCK.get_or_init(|| RwLock::new(Arc::new(SystemClock)))
}

/// Install a custom clock (typically a `FakeClock` in tests).
///
/// Safe to call multiple times; the most recent value wins.
pub fn set_clock(clock: Arc<dyn Clock>) {
    *clock_slot().write().expect("CLOCK poisoned") = clock;
}

/// Restore the default [`SystemClock`]. Useful at test teardown.
pub fn reset_clock() {
    set_clock(Arc::new(SystemClock));
}

/// Returns the current `Clock` handle (cheap `Arc` clone).
pub fn current_clock() -> Arc<dyn Clock> {
    clock_slot().read().expect("CLOCK poisoned").clone()
}

/// Convenience wrapper around `current_clock().now()` for services that
/// need a `DateTime<Utc>` but don't touch the DB. Prefer this over
/// `chrono::Utc::now()` in any code path that makes time-dependent
/// decisions (cooldowns, expiry, "last-touched" windows) — it's the only
/// entry point that honours `set_clock`, so test suites can freeze or
/// advance time by installing a `FakeClock` once and having both DB writes
/// and service logic observe the override consistently.
///
/// Rule of thumb:
///   * DB-bound timestamp → `DbInstant::now()`
///   * in-memory `DateTime<Utc>` for business logic → `now_utc()`
///   * prompt-template interpolation, log lines, etc. → either is fine;
///     keeping `now_utc()` is still cheaper than remembering where the
///     test-clock boundary sits.
pub fn now_utc() -> DateTime<Utc> {
    current_clock().now()
}

// ---------------------------------------------------------------------------
// DbInstant
// ---------------------------------------------------------------------------

/// A UTC instant that knows how to move through the SQLite boundary safely.
///
/// Prefer `DbInstant::now()` over `Utc::now()` in repo / service code so the
/// test clock has a chance to take effect. Use `DbInstant::from(dt)` to wrap
/// a `DateTime<Utc>` that came from outside (e.g. an LLM response, a
/// user-provided cutoff).
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DbInstant(DateTime<Utc>);

impl DbInstant {
    /// Current time, via the installed [`Clock`].
    pub fn now() -> Self {
        Self(current_clock().now())
    }

    /// `now - delta`, via the installed [`Clock`]. Common pattern for
    /// cooldown / lookback cutoffs in repos.
    pub fn now_minus(delta: chrono::Duration) -> Self {
        Self(current_clock().now() - delta)
    }

    /// `now + delta`, via the installed [`Clock`].
    pub fn now_plus(delta: chrono::Duration) -> Self {
        Self(current_clock().now() + delta)
    }

    /// Underlying `DateTime<Utc>` for arithmetic / display / serde elsewhere.
    pub fn into_inner(self) -> DateTime<Utc> {
        self.0
    }

    pub fn as_datetime(&self) -> DateTime<Utc> {
        self.0
    }

    /// Canonical wire format: `YYYY-MM-DDTHH:MM:SS.sssZ`.
    ///
    /// Chosen to match SQLite's `strftime('%Y-%m-%dT%H:%M:%fZ','now')` so that
    /// a row with a `DEFAULT` timestamp and a row with an app-bound
    /// `DbInstant::now()` are byte-identical.
    pub fn to_canonical_string(self) -> String {
        self.0.to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    /// Parse the canonical format or any other RFC3339 string.
    pub fn parse_rfc3339(raw: &str) -> Result<Self, chrono::ParseError> {
        DateTime::parse_from_rfc3339(raw).map(|dt| Self(dt.with_timezone(&Utc)))
    }
}

impl From<DateTime<Utc>> for DbInstant {
    fn from(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }
}

impl From<DbInstant> for DateTime<Utc> {
    fn from(value: DbInstant) -> Self {
        value.0
    }
}

impl std::fmt::Display for DbInstant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_canonical_string())
    }
}

impl ToSql for DbInstant {
    fn to_sql(&self) -> SqlResult<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.to_canonical_string()))
    }
}

impl FromSql for DbInstant {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let raw = value.as_str()?;
        match Self::parse_rfc3339(raw) {
            Ok(instant) => Ok(instant),
            Err(_) => permissive_parse(raw)
                .map(Self)
                .ok_or_else(|| FromSqlError::Other(format!("bad timestamp: {raw:?}").into())),
        }
    }
}

/// Fallback parser for timestamps written before this module existed.
///
/// Handles:
///   * `YYYY-MM-DD HH:MM:SS[.fff]` — SQLite native `datetime('now')` output
///   * `YYYY-MM-DDTHH:MM:SS[.fff]` — RFC3339 minus the timezone suffix
///
/// Anything else returns `None`. Each fallback hit emits a `warn` so the
/// bad-row backfill migration can target these rows before they disappear
/// via overwrite.
fn permissive_parse(raw: &str) -> Option<DateTime<Utc>> {
    const FORMATS: &[&str] = &[
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
    ];

    for fmt in FORMATS {
        if let Ok(naive) = NaiveDateTime::parse_from_str(raw, fmt) {
            tracing::warn!(
                raw,
                fmt,
                "legacy-format timestamp accepted via permissive fallback; backfill should replace it"
            );
            return Some(Utc.from_utc_datetime(&naive));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    fn open() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE t (ts TEXT NOT NULL)")
            .unwrap();
        c
    }

    #[test]
    fn round_trip_canonical() {
        let c = open();
        let written = DbInstant::from(
            Utc.with_ymd_and_hms(2026, 4, 22, 15, 54, 9).unwrap()
                + chrono::Duration::milliseconds(74),
        );
        c.execute("INSERT INTO t VALUES (?1)", params![written])
            .unwrap();
        let read: DbInstant = c
            .query_row("SELECT ts FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(written, read);
    }

    #[test]
    fn permissive_accepts_sqlite_native() {
        let c = open();
        // Simulate a row written by the old `datetime('now')` default.
        c.execute("INSERT INTO t VALUES ('2026-04-22 15:54:09')", [])
            .unwrap();
        let read: DbInstant = c
            .query_row("SELECT ts FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            read.as_datetime(),
            Utc.with_ymd_and_hms(2026, 4, 22, 15, 54, 9).unwrap()
        );
    }

    #[test]
    fn sql_now_matches_dbinstant_write_shape() {
        // `DbInstant::now()` and `strftime('%Y-%m-%dT%H:%M:%fZ','now')` must
        // produce strings of identical shape so the two code paths are
        // indistinguishable on read.
        let c = open();
        let sql = format!("INSERT INTO t VALUES ({SQL_NOW})");
        c.execute(&sql, []).unwrap();
        let raw: String = c
            .query_row("SELECT ts FROM t", [], |row| row.get(0))
            .unwrap();
        assert!(
            DbInstant::parse_rfc3339(&raw).is_ok(),
            "SQL_NOW produced non-RFC3339 output: {raw:?}"
        );
    }

    #[test]
    fn fake_clock_overrides_now() {
        let fixed = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        let clock = FakeClock::new(fixed);
        set_clock(Arc::new(clock.clone()));
        assert_eq!(DbInstant::now().as_datetime(), fixed);
        clock.advance(chrono::Duration::hours(1));
        assert_eq!(
            DbInstant::now().as_datetime(),
            fixed + chrono::Duration::hours(1)
        );
        reset_clock();
    }
}
