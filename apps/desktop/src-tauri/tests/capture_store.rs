use corivo_app_lib::services::capture_store::{CaptureStore, SessionStatus};
use tempfile::TempDir;

#[tokio::test]
async fn capture_store_persists_sessions_and_screenshots() {
    let temp_dir = TempDir::new().expect("temp dir");
    let store = CaptureStore::new(temp_dir.path().to_path_buf())
        .await
        .expect("store");

    let session = store.create_session(15, 5).await.expect("create session");
    assert_eq!(session.status, SessionStatus::Active);

    let (path, size) = store
        .save_screenshot(&session.id, vec![1, 2, 3, 4])
        .await
        .expect("save screenshot");
    assert!(path.exists());
    assert_eq!(size, 4);

    let listed = store.list_sessions().await.expect("list sessions");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].screenshot_count, 1);

    let screenshots = store
        .list_screenshots(&session.id)
        .await
        .expect("list screenshots");
    assert_eq!(screenshots.len(), 1);
    assert_eq!(screenshots[0].filename, "0001.jpg");

    store
        .end_session(&session.id, false)
        .await
        .expect("end session");

    let ended = store
        .get_session(&session.id)
        .await
        .expect("get session")
        .expect("session exists");
    assert_eq!(ended.status, SessionStatus::Completed);
    assert!(ended.ended_at.is_some());
}

#[tokio::test]
async fn capture_store_deletes_sessions_and_reports_storage_stats() {
    let temp_dir = TempDir::new().expect("temp dir");
    let store = CaptureStore::new(temp_dir.path().to_path_buf())
        .await
        .expect("store");

    let session = store.create_session(10, 3).await.expect("create session");
    store
        .save_screenshot(&session.id, vec![9, 8, 7])
        .await
        .expect("save screenshot");
    store
        .end_session(&session.id, false)
        .await
        .expect("end session");

    let stats = store.get_storage_stats().await.expect("storage stats");
    assert_eq!(stats.session_count, 1);
    assert_eq!(stats.screenshot_count, 1);
    assert!(stats.total_size_bytes > 0);

    let deleted = store.delete_older_than(0).await.expect("cleanup old");
    assert_eq!(deleted, 1);
    assert!(store
        .list_sessions()
        .await
        .expect("list sessions")
        .is_empty());
}
