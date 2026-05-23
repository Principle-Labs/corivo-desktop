use corivo_app_lib::db::{
    migrations::{apply_migrations, TARGET_SCHEMA_VERSION},
    pool::test_in_memory_pool,
};

/// Two pools built by `test_in_memory_pool` must not share any schema or rows.
/// Before the UUID-randomised URI, both pools aliased onto the same
/// process-global `file::memory:?cache=shared` database, which meant tests
/// executing in parallel saw each other's tables and rows. This test pins the
/// isolation contract so future refactors cannot silently re-introduce the
/// footgun.
#[test]
fn parallel_pools_have_independent_in_memory_databases() {
    let pool_a = test_in_memory_pool().expect("pool A");
    let conn_a = pool_a.get().expect("conn A");
    apply_migrations(&conn_a).expect("migrations A");

    let pool_b = test_in_memory_pool().expect("pool B");
    let conn_b = pool_b.get().expect("conn B");
    apply_migrations(&conn_b).expect("migrations B");

    // Both pools land at the latest schema version independently.
    let version_a: i64 = conn_a
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .expect("schema_version A");
    let version_b: i64 = conn_b
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .expect("schema_version B");
    assert_eq!(version_a, TARGET_SCHEMA_VERSION);
    assert_eq!(version_b, TARGET_SCHEMA_VERSION);

    // Write a sentinel into pool A's frames table (v300 SSOT).
    conn_a
        .execute(
            "INSERT INTO frames (id, captured_at, device_id, capture_session_id, \
                                  extraction_strategy) \
             VALUES ('01HSENTINEL000000000000000', \
                     '2026-04-27T10:14:23.418Z', 'dev-a', 'sess-a', 'ocr')",
            [],
        )
        .expect("insert sentinel into pool A");

    let count_a: i64 = conn_a
        .query_row("SELECT COUNT(*) FROM frames", [], |r| r.get(0))
        .expect("count A");
    assert_eq!(count_a, 1, "pool A should see its own sentinel");

    // Pool B must not see pool A's row.
    let count_b: i64 = conn_b
        .query_row("SELECT COUNT(*) FROM frames", [], |r| r.get(0))
        .expect("count B");
    assert_eq!(
        count_b, 0,
        "pool B must have an independent frames table (no leakage from pool A)"
    );
}
