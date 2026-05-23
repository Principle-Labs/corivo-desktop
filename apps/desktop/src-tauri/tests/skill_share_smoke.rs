//! Smoke test against the real host: scan ~/.agents/skills/ +
//! ~/.claude/skills/, then run sync against a temp dst and verify that
//! symlinks land where we expect.
//!
//! Skipped in CI environments where `~/.agents/skills/` doesn't exist.

use std::fs;

use corivo_app_lib::services::skill_share::SkillShareService;
use tempfile::TempDir;

#[test]
fn scan_and_sync_against_real_host() {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        eprintln!("HOME unset — skipping");
        return;
    };
    let agents = home.join(".agents/skills");
    let claude = home.join(".claude/skills");
    if !agents.is_dir() && !claude.is_dir() {
        eprintln!("no host skill sources — skipping");
        return;
    }

    let tmp = TempDir::new().expect("tempdir");
    let svc = SkillShareService::new(tmp.path().to_path_buf());

    let scanned = svc.scan();
    assert!(
        !scanned.is_empty(),
        "expected to discover at least one skill on host"
    );
    println!("discovered {} skills:", scanned.len());
    for s in &scanned {
        println!("  - {} [{:?}] -> {}", s.name, s.source, s.path.display());
    }

    // Pick the first three (or all if fewer) and ask sync to bridge them.
    let pick: Vec<String> = scanned.iter().take(3).map(|s| s.name.clone()).collect();
    svc.sync(&pick).expect("sync ok");

    let dst_root = tmp.path().join("claude-config/skills");
    for name in &pick {
        let link = dst_root.join(name);
        let meta = fs::symlink_metadata(&link).expect("symlink should exist");
        assert!(
            meta.file_type().is_symlink(),
            "{} should be a symlink",
            link.display()
        );
        let target = fs::read_link(&link).expect("read_link");
        assert!(
            target.exists(),
            "{} -> {} should resolve",
            link.display(),
            target.display()
        );
        // The link target should contain SKILL.md (i.e. it really is a
        // skill dir, not a random thing).
        assert!(
            target.join("SKILL.md").is_file(),
            "{} target missing SKILL.md",
            target.display()
        );
    }

    // Re-sync with an empty list — every previously linked skill should
    // be removed.
    svc.sync(&[]).expect("sync empty ok");
    for name in &pick {
        let link = dst_root.join(name);
        assert!(
            fs::symlink_metadata(&link).is_err(),
            "{} should be gone after empty sync",
            link.display()
        );
    }
}
