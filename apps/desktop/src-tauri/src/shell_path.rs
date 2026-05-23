//! Resolve the user's shell `PATH` and apply it to the current process.
//!
//! Desktop apps launched from Finder / Dock on macOS inherit launchd's
//! minimal `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`) instead of the user's
//! terminal `PATH` — `~/.zshrc` is only sourced by interactive login
//! shells, which `launchd` is not. Anything we (or our subprocesses,
//! including the bundled coding agent and the `bash` it spawns) try to
//! exec by name then fails to find tools installed via Homebrew, asdf,
//! cargo, bun, etc.
//!
//! At boot we ask the user's shell for its login + interactive `PATH`,
//! merge it with whatever we inherited, and `set_var("PATH", ...)` once.
//! Every later `Command::spawn` — direct, sidecar, or transitively from
//! the coding agent — inherits the fixed `PATH` automatically.

use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

const PATH_BEGIN: &str = "__CORIVO_PATH_BEGIN__";
const PATH_END: &str = "__CORIVO_PATH_END__";
const SHELL_PATH_TIMEOUT: Duration = Duration::from_secs(3);

/// Resolve a merged `PATH` suitable for `claude` and the commands it
/// spawns. If shell probing fails or times out, falls back to the app's
/// inherited `PATH`.
pub async fn resolve_for_child() -> Option<OsString> {
    let inherited = env::var_os("PATH");
    let shell_path = probe_shell_path().await;
    merge_paths(shell_path, inherited)
}

async fn probe_shell_path() -> Option<OsString> {
    #[cfg(not(unix))]
    {
        return None;
    }

    #[cfg(unix)]
    {
        let shell = env::var_os("SHELL")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/bin/zsh"));

        if !shell.is_file() {
            return None;
        }

        let script = format!("command printf '%s%s%s\\n' '{PATH_BEGIN}' \"$PATH\" '{PATH_END}'");
        let output = timeout(
            SHELL_PATH_TIMEOUT,
            Command::new(shell)
                // Login + interactive mode covers common macOS zsh setups:
                // `/etc/zprofile` / `~/.zprofile` plus `~/.zshrc`.
                .args(["-lic", &script])
                .env("CORIVO_SHELL_PATH_PROBE", "1")
                .output(),
        )
        .await
        .ok()?
        .ok()?;

        if !output.status.success() {
            return None;
        }

        extract_path_from_probe(&String::from_utf8_lossy(&output.stdout)).map(OsString::from)
    }
}

fn extract_path_from_probe(stdout: &str) -> Option<String> {
    let start = stdout.rfind(PATH_BEGIN)? + PATH_BEGIN.len();
    let rest = &stdout[start..];
    let end = rest.find(PATH_END)?;
    let path = rest[..end].trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

fn merge_paths(primary: Option<OsString>, fallback: Option<OsString>) -> Option<OsString> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for value in [primary, fallback].into_iter().flatten() {
        paths.extend(env::split_paths(&value));
    }

    let mut deduped: Vec<PathBuf> = Vec::new();
    for path in paths {
        if !deduped.iter().any(|seen| seen == &path) {
            deduped.push(path);
        }
    }

    if deduped.is_empty() {
        return None;
    }

    env::join_paths(deduped).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_sentinel_wrapped_path_after_shell_noise() {
        let stdout = "hello from rc\n__CORIVO_PATH_BEGIN__/opt/homebrew/bin:/usr/local/bin__CORIVO_PATH_END__\n";

        assert_eq!(
            extract_path_from_probe(stdout).as_deref(),
            Some("/opt/homebrew/bin:/usr/local/bin")
        );
    }

    #[test]
    fn merge_prefers_shell_path_and_deduplicates_inherited_entries() {
        let merged = merge_paths(
            Some(OsString::from("/opt/homebrew/bin:/usr/bin")),
            Some(OsString::from("/usr/bin:/bin")),
        )
        .expect("merged PATH");

        let parts: Vec<_> = env::split_paths(&merged).collect();
        assert_eq!(
            parts,
            vec![
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
            ]
        );
    }
}
