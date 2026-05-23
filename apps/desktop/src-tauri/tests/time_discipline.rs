//! CI-level enforcement that the RFC3339 timestamp discipline stays in place.
//!
//! Every time this regressed historically, the symptom was the same: a new
//! repo method shipped with `datetime('now')` baked into the SQL, which
//! writes `YYYY-MM-DD HH:MM:SS` (not RFC3339) and makes a subsequent
//! `parse_datetime` on the same column explode with "premature end of input".
//!
//! Rather than re-learn that lesson per table, this test grep-scans every
//! application source file and fails if the forbidden literal reappears
//! anywhere outside the allowlist. The allowlist is small on purpose:
//!
//!   * `src/db/time.rs` — defines the discipline and references the forbidden
//!     literal in its doc comment.
//!   * `src/db/migrations.rs` — explains the historical bug in its doc
//!     comment; referenced for context only, never executed.
//!
//! If you think you need to write `datetime('now')` somewhere new, you don't.
//! Bind a `DbInstant::now()` parameter instead, or use the `SQL_NOW` constant
//! from `crate::db::time` for schema DEFAULT clauses.

use std::fs;
use std::path::{Path, PathBuf};

/// Files where the literal `datetime('now')` is allowed to appear (always in
/// comments / docs, never in executable SQL). Paths are relative to
/// `src-tauri/`.
const ALLOWLIST: &[&str] = &[
    "src/db/time.rs",
    "src/db/migrations.rs",
    // schema.sql's header comment documents the rule (and references
    // the forbidden literal by name). Executable SQL in the file uses
    // strftime exclusively.
    "src/db/schema.sql",
    // The ban test itself must be allowed to mention what it bans.
    "tests/time_discipline.rs",
];

const FORBIDDEN: &str = "datetime('now')";

#[test]
fn no_raw_datetime_now_in_application_sql() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();

    for entry in walk(&root.join("src")).chain(walk(&root.join("tests"))) {
        let rel = entry
            .strip_prefix(root)
            .expect("walk result under manifest dir")
            .to_string_lossy()
            .replace('\\', "/");

        if ALLOWLIST.iter().any(|allowed| rel == *allowed) {
            continue;
        }

        let extension = entry
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();
        if !matches!(extension, "rs" | "sql") {
            continue;
        }

        let content = match fs::read_to_string(&entry) {
            Ok(s) => s,
            Err(_) => continue,
        };

        for (lineno, line) in content.lines().enumerate() {
            if line.contains(FORBIDDEN) {
                offenders.push(format!("{rel}:{}", lineno + 1));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "forbidden `{FORBIDDEN}` found in application sources; bind a \
         `DbInstant::now()` parameter or use `SQL_NOW` instead:\n  - {}",
        offenders.join("\n  - ")
    );
}

fn walk(root: &Path) -> Box<dyn Iterator<Item = PathBuf>> {
    let Ok(read) = fs::read_dir(root) else {
        return Box::new(std::iter::empty());
    };
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else {
            files.push(path);
        }
    }
    let subwalks = dirs.into_iter().flat_map(|d| walk(&d));
    Box::new(files.into_iter().chain(subwalks))
}
