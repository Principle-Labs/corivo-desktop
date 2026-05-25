pub mod commands;
pub mod db;
pub mod domain;
pub mod env;
pub mod error;
pub mod services;
// shell_path probes the user's interactive zsh PATH so the bundled agent's
// Bash tool can find brew / asdf / cargo / bun. Only matters on macOS where
// launchd-launched GUI apps inherit a minimal PATH; Windows app launches
// already inherit the full user PATH from the shell, so the whole module
// stays mac-only.
#[cfg(target_os = "macos")]
mod shell_path;

use std::{
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Once,
    },
};

use chrono::{Local, NaiveDate};
use tauri::{
    image::Image,
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Runtime,
};
use tauri_plugin_autostart::MacosLauncher;
use tracing_subscriber::{
    filter::LevelFilter, fmt::writer::MakeWriter, layer::SubscriberExt, util::SubscriberInitExt,
    EnvFilter, Layer,
};

use commands::{config::AppState, quick_ask::QUICK_ASK_WINDOW_LABEL};
use services::{
    capture_client::{self, CaptureClient, SpawnOptions as CaptureClientSpawnOptions},
    quick_ask_window::{
        apply_quick_ask_overlay_window_mode, hide_quick_ask, is_quick_ask_visible, show_quick_ask,
    },
};

/// Resolve the path to the bundled capture helper binary.
///
/// macOS shapes the helper as a `.app` bundle inside the parent
/// (Phase 7+, see CO-31): `LSUIElement=YES` + a stable bundle id is
/// what stops Launch Services from registering the helper as its own
/// Dock app, and what lets TCC merge its API calls into Corivo.app
/// via responsible-process attribution.
///
/// Production (macOS):
///   `Corivo.app/Contents/Resources/CorivoCaptureHelper.app/Contents/MacOS/CorivoCaptureHelper`
///   Tauri places the helper bundle there via `bundle.resources`
///   (NOT externalBin).
///
/// Dev (macOS):
///   `<repo>/apps/desktop/src-tauri/binaries/CorivoCaptureHelper.app/Contents/MacOS/CorivoCaptureHelper`
///   Same bundle layout as release; the build.sh in
///   packages/desktop-helpers/macos/ writes it there directly.
///
/// Windows: still flat. `bundle.externalBin` copies
/// `corivo-capture-helper.exe` next to the main executable.
fn locate_capture_helper_binary() -> std::result::Result<PathBuf, String> {
    let me = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let dir = me
        .parent()
        .ok_or_else(|| "current_exe has no parent".to_string())?;

    // macOS production layout: helper .app sits in Contents/Resources/.
    // current_exe() is at Contents/MacOS/Corivo, so ../Resources/.
    #[cfg(target_os = "macos")]
    {
        let prod_helper = dir
            .join("..")
            .join("Resources")
            .join("CorivoCaptureHelper.app")
            .join("Contents")
            .join("MacOS")
            .join("CorivoCaptureHelper");
        if prod_helper.is_file() {
            return Ok(prod_helper);
        }
    }

    // Windows production: flat externalBin alongside the main exec.
    #[cfg(target_os = "windows")]
    {
        let bundled = dir.join("corivo-capture-helper.exe");
        if bundled.is_file() {
            return Ok(bundled);
        }
    }

    // Dev: walk up to find src-tauri/.
    if let Some(src_tauri) = find_src_tauri_dir(dir) {
        // macOS dev: same .app layout as release, just under binaries/.
        #[cfg(target_os = "macos")]
        {
            let dev_helper = src_tauri
                .join("binaries")
                .join("CorivoCaptureHelper.app")
                .join("Contents")
                .join("MacOS")
                .join("CorivoCaptureHelper");
            if dev_helper.is_file() {
                return Ok(dev_helper);
            }
        }

        // Windows dev: triple-suffixed flat binary.
        #[cfg(target_os = "windows")]
        {
            let triple = host_target_triple();
            let candidate = src_tauri
                .join("binaries")
                .join(format!("corivo-capture-helper-{triple}.exe"));
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    Err(format!(
        "no capture helper binary near {} \
         (macOS prod: Contents/Resources/CorivoCaptureHelper.app; \
         macOS dev: src-tauri/binaries/CorivoCaptureHelper.app; \
         Windows: corivo-capture-helper(.exe))",
        dir.display()
    ))
}

/// Walk up from `start` until we find a directory that looks like
/// `src-tauri/` (has both `Cargo.toml` and `binaries/` subdir).
fn find_src_tauri_dir(start: &Path) -> Option<PathBuf> {
    let mut p: &Path = start;
    loop {
        if p.join("Cargo.toml").is_file() && p.join("binaries").is_dir() {
            return Some(p.to_path_buf());
        }
        p = p.parent()?;
    }
}

#[cfg(target_os = "windows")]
fn host_target_triple() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64-pc-windows-msvc"
    } else {
        "unknown-target"
    }
}

/// Spawn the capture helper sidecar and complete its hello handshake.
/// Synchronous wrapper around the async `CaptureClient::spawn` because
/// `setup` runs on a non-async thread.
fn spawn_capture_helper() -> std::result::Result<CaptureClient, String> {
    let path = locate_capture_helper_binary()?;
    tracing::info!(?path, "capture_helper.locate_resolved");
    let opts = CaptureClientSpawnOptions::new(path, env!("CARGO_PKG_VERSION"));
    tauri::async_runtime::block_on(CaptureClient::spawn(opts)).map_err(|e| format!("spawn: {e}"))
}
use db::{
    repos::{
        chat::{SqliteChatMessageRepo, SqliteChatThreadRepo},
        frames::SqliteFrameRepo,
        notes::SqliteNotesRepo,
    },
    Database,
};
use services::{
    background_agent_task::BackgroundAgentScheduler,
    capture_pipeline::{CapturePipeline, CapturePipelineConfig},
    capture_store::CaptureStore,
    config_service::ConfigService,
    connector::ConnectorRegistry,
    double_tap_hotkey::DoubleTapHotkey,
    exclusion::ExclusionEngine,
    exec_agent::{load_local_context, McpBridge},
    extractor::AdapterRegistry,
    foreground_monitor::ForegroundAppMonitor,
    hotkey::HotkeyService,
    macos_system_surface::{
        apply_macos_system_surface_mode, sync_macos_system_surface_from_runtime,
        MacOSSystemSurfaceMode, DEFAULT_MAIN_WINDOW_LABEL,
    },
    retention::RetentionTask,
    scheduled_workflows::{ScheduledWorkflowTicker, WorkflowStore},
    skill_share::SkillShareService,
    snapshot_consumer::LocalConsumer,
};

static TRACING_INIT: Once = Once::new();
const MAIN_WINDOW_LABEL: &str = DEFAULT_MAIN_WINDOW_LABEL;
const TRAY_ICON_ID: &str = "corivo-menubar";
const TRAY_OPEN_MENU_ID: &str = "tray-open";
const TRAY_QUIT_MENU_ID: &str = "tray-quit";
const TRAY_PAUSE_HOUR_ID: &str = "tray-pause-hour";
const TRAY_PAUSE_DAY_ID: &str = "tray-pause-day";
const TRAY_RESUME_ID: &str = "tray-resume";
const PAUSE_HOUR_SECONDS: i64 = 60 * 60;
const PAUSE_DAY_SECONDS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloseRequestAction {
    HideToTray,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayMenuAction {
    OpenMainWindow,
    QuitApp,
    PauseHour,
    PauseDay,
    Resume,
}

#[derive(Default)]
struct AppLifecycleControl {
    quitting: AtomicBool,
    runtime_shutdown_started: AtomicBool,
}

#[derive(Clone, Debug)]
struct DailyLogFileWriter {
    log_dir: PathBuf,
}

impl DailyLogFileWriter {
    fn new(log_dir: PathBuf) -> Self {
        Self { log_dir }
    }
}

impl<'a> MakeWriter<'a> for DailyLogFileWriter {
    type Writer = Box<dyn io::Write + Send>;

    fn make_writer(&'a self) -> Self::Writer {
        match open_dated_log_file(&self.log_dir, Local::now().date_naive()) {
            Ok(file) => Box::new(file),
            Err(error) => {
                eprintln!("failed to open corivo log file: {error}");
                Box::new(io::sink())
            }
        }
    }
}

fn ensure_log_directory(app_data_dir: &Path) -> io::Result<PathBuf> {
    let log_dir = app_data_dir.join("logs");
    fs::create_dir_all(&log_dir)?;
    Ok(log_dir)
}

fn dated_log_file_path(log_dir: &Path, date: NaiveDate) -> PathBuf {
    log_dir.join(format!("{}.log", date.format("%Y-%m-%d")))
}

fn open_dated_log_file(log_dir: &Path, date: NaiveDate) -> io::Result<std::fs::File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dated_log_file_path(log_dir, date))
}

/// Read or generate the per-machine `device_id` (spec §四). Stored as a
/// flat file under `$APPDATA/device.id`; not a secret — losing it just
/// rotates a UUID.
fn ensure_device_id(app_data_dir: &Path) -> error::Result<String> {
    let path = app_data_dir.join("device.id");
    if path.exists() {
        if let Ok(raw) = fs::read_to_string(&path) {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&path, &id).map_err(|error| {
        error::CorivoError::Internal(format!("failed to persist device.id: {error}"))
    })?;
    Ok(id)
}

fn init_tracing(app_data_dir: &Path) {
    TRACING_INIT.call_once(|| {
        let log_dir = ensure_log_directory(app_data_dir)
            .unwrap_or_else(|error| panic!("failed to initialize log directory: {error}"));

        let console_layer = tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_filter(business_console_filter());
        let file_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_target(false)
            .with_writer(DailyLogFileWriter::new(log_dir))
            .with_filter(LevelFilter::INFO);

        // tracing::error! → Sentry event；warn!/info! → breadcrumb（与 error
        // 一起打包上报，不会单独触发告警）。debug/trace 直接丢弃避免噪声。
        let sentry_layer =
            sentry::integrations::tracing::layer().event_filter(|md| match *md.level() {
                tracing::Level::ERROR => sentry::integrations::tracing::EventFilter::Event,
                tracing::Level::WARN | tracing::Level::INFO => {
                    sentry::integrations::tracing::EventFilter::Breadcrumb
                }
                _ => sentry::integrations::tracing::EventFilter::Ignore,
            });

        tracing_subscriber::registry()
            .with(console_layer)
            .with(file_layer)
            .with(sentry_layer)
            .init();
    });
}

fn business_console_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "debug,\
             hyper=warn,\
             hyper_util=warn,\
             reqwest=warn,\
             h2=warn,\
             rustls=warn,\
             tungstenite=warn,\
             tokio_tungstenite=warn,\
             tower=warn,\
             tao=warn,\
             wry=warn,\
             tauri=info,\
             tauri_plugin_updater=info,\
             tauri_plugin_store=info,\
             tauri_plugin_http=info,\
             r2d2=info",
        )
    })
}

fn resolve_close_request_action(
    config: &crate::domain::config::Config,
    is_quitting: bool,
) -> CloseRequestAction {
    if is_quitting || !config.app.minimize_to_tray {
        CloseRequestAction::Shutdown
    } else {
        CloseRequestAction::HideToTray
    }
}

fn parse_tray_menu_action(menu_id: &str) -> Option<TrayMenuAction> {
    match menu_id {
        TRAY_OPEN_MENU_ID => Some(TrayMenuAction::OpenMainWindow),
        TRAY_QUIT_MENU_ID => Some(TrayMenuAction::QuitApp),
        TRAY_PAUSE_HOUR_ID => Some(TrayMenuAction::PauseHour),
        TRAY_PAUSE_DAY_ID => Some(TrayMenuAction::PauseDay),
        TRAY_RESUME_ID => Some(TrayMenuAction::Resume),
        _ => None,
    }
}

fn is_app_quitting<R: Runtime>(app_handle: &AppHandle<R>) -> bool {
    app_handle
        .try_state::<AppLifecycleControl>()
        .map(|state| state.quitting.load(Ordering::SeqCst))
        .unwrap_or(false)
}

fn mark_app_quitting<R: Runtime>(app_handle: &AppHandle<R>) {
    if let Some(state) = app_handle.try_state::<AppLifecycleControl>() {
        state.quitting.store(true, Ordering::SeqCst);
    }
}

fn shutdown_runtime_once<R: Runtime>(app_handle: &AppHandle<R>) {
    if let Some(control) = app_handle.try_state::<AppLifecycleControl>() {
        if control
            .runtime_shutdown_started
            .swap(true, Ordering::SeqCst)
        {
            return;
        }
    }

    if let Some(state) = app_handle.try_state::<AppState>() {
        if let Err(error) = tauri::async_runtime::block_on(state.capture_store.shutdown()) {
            tracing::error!("CaptureStore shutdown failed: {error:?}");
        }
        if let Err(error) = state.db.shutdown() {
            tracing::error!("DB shutdown failed: {error:?}");
        }
    }
}

fn request_app_quit<R: Runtime>(app_handle: &AppHandle<R>) {
    mark_app_quitting(app_handle);
    shutdown_runtime_once(app_handle);
    app_handle.exit(0);
}

fn show_main_window<R: Runtime>(app_handle: &AppHandle<R>) {
    let _ = apply_macos_system_surface_mode(app_handle, MacOSSystemSurfaceMode::RegularForeground);
    if let Some(window) = app_handle.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

const TRAY_ICON_BYTES: &[u8] = include_bytes!("../icons/menubar.png");

fn load_tray_icon() -> error::Result<Image<'static>> {
    let decoded = image::load_from_memory(TRAY_ICON_BYTES).map_err(|error| {
        error::CorivoError::Config(format!("Could not decode tray icon: {error}"))
    })?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(Image::new_owned(rgba.into_raw(), width, height))
}

/// Build the tray menu reflecting the current pause state. When
/// paused, the two "Pause for N" rows collapse into a single "Resume"
/// row labeled with the local-time expiry so the user can see exactly
/// when capture comes back on its own.
fn build_tray_menu<R: Runtime>(
    app_handle: &AppHandle<R>,
    paused_until: Option<chrono::DateTime<chrono::Utc>>,
) -> error::Result<tauri::menu::Menu<R>> {
    let mut builder = MenuBuilder::new(app_handle)
        .text(TRAY_OPEN_MENU_ID, "Open Corivo")
        .separator();
    builder = if let Some(until) = paused_until {
        let local = until.with_timezone(&chrono::Local);
        builder.text(
            TRAY_RESUME_ID,
            format!("Resume (paused until {})", local.format("%H:%M")),
        )
    } else {
        builder
            .text(TRAY_PAUSE_HOUR_ID, "Pause for 1 hour")
            .text(TRAY_PAUSE_DAY_ID, "Pause for 1 day")
    };
    builder
        .separator()
        .text(TRAY_QUIT_MENU_ID, "Quit Corivo")
        .build()
        .map_err(|error| {
            error::CorivoError::Internal(format!("Could not build tray menu: {error}"))
        })
}

/// Swap the tray's menu to reflect a fresh pause snapshot. Idempotent —
/// safe to call on every pause/resume edge. Reads pause state from the
/// pipeline currently mounted on `AppState`.
async fn refresh_tray_menu<R: Runtime>(app_handle: &AppHandle<R>) {
    let Some(tray) = app_handle.tray_by_id(TRAY_ICON_ID) else {
        return;
    };
    let paused_until = match app_handle.try_state::<AppState>() {
        Some(state) => match state.capture_pipeline.as_ref() {
            Some(pipeline) => pipeline.paused_until().await,
            None => None,
        },
        None => None,
    };
    match build_tray_menu(app_handle, paused_until) {
        Ok(menu) => {
            if let Err(error) = tray.set_menu(Some(menu)) {
                tracing::warn!(?error, "tray.set_menu_failed");
            }
        }
        Err(error) => tracing::warn!(?error, "tray.rebuild_menu_failed"),
    }
}

/// Run a pause/resume tray action: call the pipeline, persist the
/// expiry in `Config`, and rebuild the tray menu. The tray callback
/// dispatches into this on a tokio task because tray events fire on
/// the main thread (sync), but the pipeline + config-service APIs are
/// async.
async fn run_tray_pause_action<R: Runtime>(app_handle: AppHandle<R>, action: TrayMenuAction) {
    let Some(state) = app_handle.try_state::<AppState>() else {
        tracing::warn!("tray.pause_action_before_state_ready");
        return;
    };
    let Some(pipeline) = state.capture_pipeline.as_ref() else {
        tracing::warn!("tray.pause_action_pipeline_missing");
        return;
    };
    let pipeline = pipeline.clone();
    let config_service = state.config_service.clone();
    drop(state);

    let mut next_paused_until: Option<chrono::DateTime<chrono::Utc>> = None;
    let result = match action {
        TrayMenuAction::PauseHour | TrayMenuAction::PauseDay => {
            let seconds = if matches!(action, TrayMenuAction::PauseHour) {
                PAUSE_HOUR_SECONDS
            } else {
                PAUSE_DAY_SECONDS
            };
            let until = chrono::Utc::now() + chrono::Duration::seconds(seconds);
            next_paused_until = Some(until);
            pipeline.pause_until(until).await
        }
        TrayMenuAction::Resume => pipeline.resume().await,
        _ => return,
    };

    if let Err(error) = result {
        tracing::warn!(?error, ?action, "tray.pause_action_failed");
    } else {
        let mut persisted = config_service.get();
        if persisted.app.capture_paused_until != next_paused_until {
            persisted.app.capture_paused_until = next_paused_until;
            if let Err(error) = config_service.update(persisted) {
                tracing::warn!(?error, "tray.pause_action_persist_failed");
            }
        }
    }

    refresh_tray_menu(&app_handle).await;

    // Frontends that mirror `capture_status` should re-fetch — emit the
    // same lightweight signal the config write would have, so React
    // Query's existing `["config"]` invalidate flow picks the new value
    // up. We piggyback on the config-changed event rather than adding a
    // new one because the panel that needs to react is already wired to
    // it.
    if let Err(error) = app_handle.emit(crate::domain::config::ConfigChanged::EVENT, &()) {
        tracing::warn!(?error, "tray.pause_action_emit_failed");
    }
}

fn setup_menu_bar_preview<R: Runtime>(app_handle: &AppHandle<R>) -> error::Result<()> {
    if app_handle.tray_by_id(TRAY_ICON_ID).is_some() {
        return Ok(());
    }

    // Reflect any already-installed pause state (restored from
    // `Config.app.capture_paused_until` earlier in `setup`) on first
    // paint so the menu doesn't briefly flash "Pause for 1 hour"
    // before the rebuild catches up.
    let initial_paused_until = app_handle
        .try_state::<AppState>()
        .and_then(|state| state.capture_pipeline.as_ref().cloned())
        .and_then(|pipeline| {
            tauri::async_runtime::block_on(async move { pipeline.paused_until().await })
        });
    let tray_menu = build_tray_menu(app_handle, initial_paused_until)?;

    let tray_icon = load_tray_icon()?;

    TrayIconBuilder::with_id(TRAY_ICON_ID)
        .menu(&tray_menu)
        .icon(tray_icon)
        .tooltip("Corivo")
        .icon_as_template(true)
        .show_menu_on_left_click(false)
        .on_menu_event(
            |app, event| match parse_tray_menu_action(event.id().as_ref()) {
                Some(TrayMenuAction::OpenMainWindow) => show_main_window(app),
                Some(TrayMenuAction::QuitApp) => request_app_quit(app),
                Some(
                    action @ (TrayMenuAction::PauseHour
                    | TrayMenuAction::PauseDay
                    | TrayMenuAction::Resume),
                ) => {
                    let app_for_task = app.clone();
                    tauri::async_runtime::spawn(async move {
                        run_tray_pause_action(app_for_task, action).await;
                    });
                }
                None => {}
            },
        )
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app_handle)
        .map_err(|error| {
            error::CorivoError::Internal(format!("Could not create tray icon: {error}"))
        })?;

    Ok(())
}

/// Quick Ask global-hotkey entrypoint (spec §六).
///
/// Two-phase open flow so window visibility doesn't wait on the AX walk.
///
/// Phase A (probe + screenshot, ~30–110 ms) runs **before** `window.show()`
/// because:
///   - `foreground::probe()` (NSWorkspace.frontmostApplication) MUST run
///     before show — otherwise it returns Corivo itself.
///   - the screenshot MUST be taken before the overlay appears, or the
///     OCR fallback would see Corivo's translucent panel instead of the
///     user's app.
/// Once phase A is done we show the window and emit `quick-ask:opened`
/// with a [`QuickAskSkeleton`]; the FocusCard renders the title row
/// straight away.
///
/// Phase B (per-app adapter pipeline + frame ingest, 50–1500 ms) is
/// spawned on a tokio task **after** `window.show()`. When it lands it
/// fires `quick-ask:focus-ready` with the full [`FocusContext`] —
/// FocusCard upgrades from skeleton to the full preview, and the Send
/// button (gated on `focus.frame_id`) becomes usable.
///
/// **Threading note**: every `WebviewWindow::{is_visible,show,hide,set_focus}`
/// call below goes through `run_on_main_thread` because AppKit
/// requires window operations on the main thread. Calling them from
/// a tokio worker leaks an `NSException` through the FFI boundary
/// and the runtime aborts with "Rust cannot catch foreign exceptions"
/// — the same trap the notification overlay panel guards against in
/// `providers/notification/panel.rs`.
fn on_quick_ask_hotkey<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    let t_hotkey = std::time::Instant::now();
    tracing::info!("quick_ask.hotkey.t0_received");
    tauri::async_runtime::spawn(async move {
        // Toggle: a second hotkey press while the panel is up dismisses it.
        let toggle_start = std::time::Instant::now();
        if hide_quick_ask_window_if_visible(&app).await {
            tracing::info!(
                elapsed_ms = toggle_start.elapsed().as_millis() as u64,
                "quick_ask.hotkey.toggle_hid"
            );
            return;
        }
        tracing::info!(
            elapsed_ms = toggle_start.elapsed().as_millis() as u64,
            total_ms = t_hotkey.elapsed().as_millis() as u64,
            "quick_ask.hotkey.t1_toggle_check_done"
        );

        let state = match app.try_state::<AppState>() {
            Some(s) => s,
            None => {
                tracing::warn!("quick_ask.hotkey_fired_before_state_ready");
                return;
            }
        };

        // Closed-beta gate: Quick Ask is a chat surface, so a
        // logged-out user pressing the hotkey would just hit "no
        // credentials" mid-prompt. Surface the main window and let
        // the front-end's `corivo:auth_required` listener swap to
        // /login instead. Mirrors the per-route guard in
        // `routes/__root.tsx`.
        let must_login =
            state.cloud.auth.is_available() && !state.cloud.session.has_session().unwrap_or(false);
        if must_login {
            tracing::info!("quick_ask.hotkey.skipped_logged_out");
            if let Some(window) = app.get_webview_window("main") {
                let win = window.clone();
                let _ = app.run_on_main_thread(move || {
                    let _ = win.show();
                    let _ = win.unminimize();
                    let _ = win.set_focus();
                });
            }
            let _ = app.emit("corivo:auth_required", ());
            return;
        }

        let pipeline = match state.capture_pipeline.as_ref() {
            Some(p) => p.clone(),
            None => {
                tracing::warn!("quick_ask.capture_pipeline_missing");
                return;
            }
        };

        // Drop the State guard before any await to keep us off the
        // tokio runtime's `Send` complaints — `State` borrows the app.
        drop(state);

        // Menubar pause freezes all screen reading, Quick Ask
        // included — phase A would screenshot + AX-walk the
        // foreground app, exactly what the user paused us from doing.
        // Bail before showing the panel so we don't tease them with an
        // open overlay that can't actually answer anything.
        if pipeline.is_paused() {
            tracing::info!("quick_ask.skipped_paused");
            return;
        }
        tracing::info!(
            total_ms = t_hotkey.elapsed().as_millis() as u64,
            "quick_ask.hotkey.t2_state_resolved"
        );

        // Type-to-create: we no longer mint an empty `chat_threads` row
        // up-front. The frontend's `useChatStream` does it lazily on the
        // first send (matches `/ask`'s flow). Hotkey path now has just
        // one async step — phase A.
        let phase_a_start = std::time::Instant::now();
        let phase_a = match pipeline.invoke_quick_ask_phase_a().await {
            Ok(p) => p,
            Err(error) => {
                tracing::error!(?error, "quick_ask.phase_a_failed");
                emit_quick_ask_error(&app, format!("Could not read window contents: {error}"));
                show_quick_ask_window(&app).await;
                return;
            }
        };
        tracing::info!(
            elapsed_ms = phase_a_start.elapsed().as_millis() as u64,
            total_ms = t_hotkey.elapsed().as_millis() as u64,
            "quick_ask.hotkey.t3_phase_a_done"
        );

        // User-perceived "open" happens here.
        let show_start = std::time::Instant::now();
        show_quick_ask_window(&app).await;
        tracing::info!(
            elapsed_ms = show_start.elapsed().as_millis() as u64,
            total_ms = t_hotkey.elapsed().as_millis() as u64,
            "quick_ask.hotkey.t4_window_shown"
        );

        match phase_a {
            crate::services::capture_pipeline::QuickAskPhaseAOutcome::Excluded(focus) => {
                // Excluded path has no work left — emit the full payload.
                let payload = serde_json::json!({
                    "kind": "ready",
                    "focus": focus,
                });
                if let Err(error) = app.emit_to(QUICK_ASK_WINDOW_LABEL, "quick-ask:opened", payload)
                {
                    tracing::warn!(?error, "quick_ask.emit_opened_failed");
                }
                tracing::info!(
                    total_ms = t_hotkey.elapsed().as_millis() as u64,
                    "quick_ask.hotkey.t5_emit_done_excluded"
                );
            }
            crate::services::capture_pipeline::QuickAskPhaseAOutcome::Pending {
                skeleton,
                state,
            } => {
                let payload = serde_json::json!({
                    "kind": "pending",
                    "skeleton": skeleton,
                });
                if let Err(error) = app.emit_to(QUICK_ASK_WINDOW_LABEL, "quick-ask:opened", payload)
                {
                    tracing::warn!(?error, "quick_ask.emit_opened_failed");
                }
                tracing::info!(
                    total_ms = t_hotkey.elapsed().as_millis() as u64,
                    "quick_ask.hotkey.t5_emit_done_pending"
                );

                // Phase B in the background: AX walk + adapter pipeline +
                // frame ingest. When done, emit the full FocusContext so
                // the FocusCard upgrades and Send unlocks.
                let app_for_phase_b = app.clone();
                let pipeline_for_phase_b = pipeline.clone();
                tauri::async_runtime::spawn(async move {
                    let phase_b_start = std::time::Instant::now();
                    match pipeline_for_phase_b.invoke_quick_ask_phase_b(state).await {
                        Ok(focus) => {
                            tracing::info!(
                                elapsed_ms = phase_b_start.elapsed().as_millis() as u64,
                                "quick_ask.hotkey.t6_phase_b_done"
                            );
                            if let Err(error) = app_for_phase_b.emit_to(
                                QUICK_ASK_WINDOW_LABEL,
                                "quick-ask:focus-ready",
                                focus,
                            ) {
                                tracing::warn!(?error, "quick_ask.emit_focus_ready_failed");
                            }
                        }
                        Err(error) => {
                            tracing::error!(?error, "quick_ask.phase_b_failed");
                            emit_quick_ask_error(
                                &app_for_phase_b,
                                format!("Could not read window contents: {error}"),
                            );
                        }
                    }
                });
            }
        }
    });
}

/// Run a closure on the macOS main thread and await its result. Mirrors
/// the pattern in `providers/notification/panel.rs`. Uses oneshot
/// instead of std::sync::mpsc because we're already in an async ctx.
async fn run_on_main<R, F, T>(app: &AppHandle<R>, task: F) -> T
where
    R: Runtime,
    F: FnOnce(AppHandle<R>) -> T + Send + 'static,
    T: Send + 'static,
{
    let app_for_task = app.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Err(error) = app.run_on_main_thread(move || {
        let _ = tx.send(task(app_for_task));
    }) {
        // Tauri runtime is shutting down; nothing we can do but bail.
        // Returning a panic here would leave callers with no signal,
        // so fall back to running the closure inline (best-effort —
        // if AppKit is involved this may still abort, but at least
        // the error is logged).
        panic!("run_on_main: failed to dispatch to main thread: {error}");
    }
    rx.await
        .expect("run_on_main: main-thread task dropped result")
}

/// Show + focus the Quick Ask window on the main thread.
async fn show_quick_ask_window<R: Runtime>(app: &AppHandle<R>) {
    use commands::quick_ask::{QUICK_ASK_INITIAL_HEIGHT, QUICK_ASK_WIDTH};
    let dispatch_start = std::time::Instant::now();
    run_on_main(app, move |app| {
        let main_thread_start = std::time::Instant::now();
        tracing::info!(
            dispatch_ms = dispatch_start.elapsed().as_millis() as u64,
            "quick_ask.show.main_thread_entered"
        );
        if let Some(window) = app.get_webview_window(QUICK_ASK_WINDOW_LABEL) {
            apply_quick_ask_overlay_window_mode(&window);
            // Reset to the initial (empty-transcript) footprint before
            // showing — the frontend calls `stream.reset()` on
            // `quick-ask:opened`, then a `ResizeObserver` immediately
            // refines the height once the shell has laid out. Pre-set
            // here so the first paint isn't at the previous expanded
            // height.
            let _ = window.set_size(tauri::LogicalSize::new(
                QUICK_ASK_WIDTH,
                QUICK_ASK_INITIAL_HEIGHT,
            ));
            let s1 = std::time::Instant::now();
            // Single panel call replaces the old `show()` + `set_focus()`
            // pair. NSPanel's NonactivatingPanel style mask means
            // makeKeyWindow does not bring Corivo to the foreground,
            // and Floating-level + FullScreenAuxiliary lets the panel
            // float above other apps' fullscreen Spaces while staying
            // below the IME candidate window's level.
            show_quick_ask(&window);
            tracing::info!(
                elapsed_ms = s1.elapsed().as_millis() as u64,
                main_thread_total_ms = main_thread_start.elapsed().as_millis() as u64,
                "quick_ask.show.panel_show_done"
            );
        } else {
            tracing::warn!("quick_ask.window_missing_at_show");
        }
    })
    .await;
}

/// Toggle helper: returns `true` if the window was visible and we
/// just hid it (caller should then bail out of the open flow).
async fn hide_quick_ask_window_if_visible<R: Runtime>(app: &AppHandle<R>) -> bool {
    run_on_main(app, |app| {
        if let Some(window) = app.get_webview_window(QUICK_ASK_WINDOW_LABEL) {
            if is_quick_ask_visible(&window) {
                hide_quick_ask(&window);
                return true;
            }
        }
        false
    })
    .await
}

fn emit_quick_ask_error<R: Runtime>(app: &AppHandle<R>, message: String) {
    if let Err(error) = app.emit_to(QUICK_ASK_WINDOW_LABEL, "quick-ask:error", message) {
        tracing::warn!(?error, "quick_ask.emit_error_failed");
    }
}

fn with_platform_plugins<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());
    builder
}

/// Apply NSVisualEffectView vibrancy to the Quick Ask window so the
/// background uses macOS's native blur instead of CSS `backdrop-filter`.
/// The window is created up-front by `tauri.conf.json` (`visible: false`),
/// so `get_webview_window` succeeds during `setup`.
///
/// `QUICK_ASK_CORNER_RADIUS` MUST stay in sync with
/// `.quick-ask-shell { border-radius }` in
/// `src/styles/quick-ask.css` — otherwise the native vibrancy mask and
/// the React shell render at different radii and the corners look
/// stepped.
#[cfg(target_os = "macos")]
const QUICK_ASK_CORNER_RADIUS: f64 = 18.0;

fn apply_quick_ask_vibrancy<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
        let Some(window) = app.get_webview_window(QUICK_ASK_WINDOW_LABEL) else {
            tracing::warn!("quick_ask.vibrancy.window_missing");
            return;
        };
        if let Err(error) = apply_vibrancy(
            &window,
            NSVisualEffectMaterial::HudWindow,
            Some(NSVisualEffectState::Active),
            Some(QUICK_ASK_CORNER_RADIUS),
        ) {
            tracing::warn!(?error, "quick_ask.vibrancy.apply_failed");
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Sentry 必须在所有可能 panic 的代码之前初始化。guard 绑到一个真实
    // 命名的变量上，drop 时会 flush in-flight events（Tauri 的 run loop
    // 不返回直到 app 退出，所以 guard 自然活到进程结束）。`_` 前缀仅是
    // "未使用"标记，不会立即 drop。
    //
    // 仅在 corivo-cloud 构建里初始化——Sentry DSN 指向 Corivo 自己的
    // 项目，开源构建不应该把 panic / tracing::error 漏到那里。OSS 路径
    // 走"显式 no-op"：未 init 的 `sentry::capture_*` 调用本身就是空操作，
    // 不需要在每个 callsite 加 cfg。
    #[cfg(feature = "corivo-cloud")]
    let _sentry_guard = sentry::init((
        crate::services::cloud::corivo::SENTRY_DSN,
        sentry::ClientOptions {
            release: sentry::release_name!(),
            // 隐私姿态：Sentry SDK 不自动捕获 PII（用户名 / IP / cookie 等）。
            // 排障需要联系用户时，让用户主动通过 Settings → 反馈 提供上下文，
            // 不在每个 event 默认带 PII。参见 docs/privacy-filter-spec.md。
            send_default_pii: false,
            environment: Some(
                if cfg!(debug_assertions) { "development" } else { "production" }.into(),
            ),
            ..Default::default()
        },
    ));

    let builder = with_platform_plugins(tauri::Builder::default());
    // single-instance MUST be the first plugin so a second launch
    // (triggered by a `corivo://` deep link when the app is already
    // running) is forwarded into the existing process instead of
    // spawning a duplicate. The closure runs in the FIRST process when
    // a second instance starts; we bring the main window forward and
    // let the deep-link plugin's own listener handle URL routing.
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_deep_link::init());

    #[cfg(feature = "corivo-cloud")]
    let builder = crate::services::cloud::private_hooks::with_tauri_plugins(builder);

    builder
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // v1432 — macOS banner for scheduled-workflow runs. The user
        // gets a system prompt the first time we call `.show()`; later
        // calls are silent until the user explicitly revokes.
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // Register the deep-link handler before anything else in
            // setup runs — `on_open_url` is callback-based and the
            // plugin queues any URLs that arrive before this point, so
            // ordering inside setup() doesn't actually matter, but
            // putting it up top keeps the wiring obvious.
            //
            // Today the only thing we do with deep links is bring the
            // main window forward (the OAuth web page's "返回 Corivo"
            // button hits `corivo://activate`). We deliberately do
            // NOT pass `code`, `id_token`, or any session data through
            // deep links — token exchange stays inside the loopback
            // flow so nothing sensitive crosses the URL boundary.
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    tracing::info!(urls = ?event.urls(), "deep_link.received");
                    show_main_window(&handle);
                });
            }

            let app_data_dir = app.path().app_data_dir().map_err(|error| {
                error::CorivoError::Config(format!("Could not resolve app data dir: {error}"))
            })?;
            init_tracing(&app_data_dir);

            // Recover the user's interactive-shell PATH before any
            // service spawns subprocesses. Apps launched from Finder /
            // Dock on macOS only inherit launchd's minimal PATH; without
            // this fix, the bundled agent's Bash tool can't find bun,
            // node, git, brew-installed binaries, etc. Set once here →
            // every later Command::spawn inherits the merged PATH.
            #[cfg(target_os = "macos")]
            if let Some(merged) = tauri::async_runtime::block_on(shell_path::resolve_for_child()) {
                tracing::info!(
                    path = ?merged,
                    "shell_path.applied_to_process"
                );
                std::env::set_var("PATH", merged);
            }

            let db = Arc::new(Database::initialize(app_data_dir.clone())?);
            let capture_store = Arc::new(
                tauri::async_runtime::block_on(CaptureStore::new(app_data_dir.clone())).map_err(
                    |error| {
                        error::CorivoError::Internal(format!("CaptureStore init failed: {error}"))
                    },
                )?,
            );
            let config_service =
                Arc::new(ConfigService::new(app.handle()).expect("failed to init ConfigService"));

            // jieba dictionary warm-up so the first FTS query (Phase 4+
            // hooks this into frames_fts) doesn't pay the 200-500 ms
            // initialisation cost synchronously.
            tauri::async_runtime::spawn_blocking(crate::services::tokenize::warm_up);

            // Stable per-machine device id (spec §四 envelope.device_id).
            // Stored as a flat file under app data so reinstalls keep
            // identity until the user wipes the dir.
            let device_id = ensure_device_id(
                capture_store
                    .captures_root()
                    .parent()
                    .unwrap_or(std::path::Path::new(".")),
            )?;

            // v3 frames repo + capture pipeline wiring.
            let frames_repo: Arc<dyn crate::db::repos::frames::FrameRepo> =
                Arc::new(SqliteFrameRepo::new(db.pool()));

            // LocalConsumer gets the AppHandle so each ingested frame
            // can fire `capture:frame-ingested`. Quick Ask's overlay
            // listens for that event and auto-updates its FocusCard
            // whenever the user switches apps — no manual refresh.
            let consumer: Arc<dyn crate::services::snapshot_consumer::SnapshotConsumer> = Arc::new(
                LocalConsumer::with_app(frames_repo.clone(), app.handle().clone()),
            );

            // CO-31: register Corivo.app as the responsible process for
            // Accessibility BEFORE the helper sidecar starts.
            //
            // TCC routes Accessibility (and most other categories) up the
            // responsible-process chain — but only if the parent is
            // already in tccd's known-clients table. Calling
            // AXIsProcessTrusted() with no prompt is a cheap way to put
            // ai.corivo.desktop in that table; once it's there, every AX
            // request the helper makes (foreground watch, AX text walks,
            // selection probe) gets attributed back to Corivo and shows
            // up under "Corivo" in System Settings → Privacy &
            // Security → Accessibility, not as a separate "Corivo
            // Capture Helper" entry.
            //
            // Returns a bool we discard. The actual permission state is
            // surfaced via the helper's permission_status RPC, see
            // services::extractor::ax_extractor::is_process_trusted().
            #[cfg(target_os = "macos")]
            unsafe {
                let _ = accessibility_sys::AXIsProcessTrusted();
            }

            // Phase 6: spawn the cross-platform capture helper sidecar
            // BEFORE wiring anything that depends on it. Best-effort — if
            // the binary isn't present (fresh checkout before
            // `pnpm --filter @corivo/desktop-helpers build` runs) or the handshake fails, log
            // + degrade rather than aborting boot. The foreground monitor
            // and capture pipeline both read `capture_client::global`, so
            // they're conditional on a successful spawn.
            let capture_client = match spawn_capture_helper() {
                Ok(client) => {
                    let arc = Arc::new(client);
                    capture_client::global::install(arc.clone());
                    Some(arc)
                }
                Err(error) => {
                    tracing::warn!(?error, "capture_helper.spawn_failed_degraded_mode");
                    let _ = app.handle().emit("capture_helper:degraded", ());
                    None
                }
            };

            let mut cfg_now = config_service.get();

            // Foreground-app monitor: installed when the helper is up so
            // the Quick Ask overlay tracks app switches even when capture
            // is paused. The capture pipeline subscribes to it on `start`
            // and unsubscribes on `stop`. In degraded mode (helper
            // missing) we skip both monitor and pipeline.
            let capture_pipeline: Option<Arc<CapturePipeline>> = match capture_client
                .as_ref()
                .map(|_| ForegroundAppMonitor::install(app.handle().clone()))
            {
                Some(Ok(foreground_monitor)) => {
                    let pipeline_config = CapturePipelineConfig {
                        interval_secs: cfg_now.capture.interval_secs.max(1),
                        jpeg_quality: cfg_now.capture.jpeg_quality,
                        idle_skip_secs: 60,
                    };

                    // Phase 5 per-app adapter registry. Currently ships
                    // with GenericAxAdapter only; per-app adapters land
                    // in Batch 12.
                    let adapter_registry = Arc::new(AdapterRegistry::builtin());

                    Some(Arc::new(CapturePipeline::new(
                        consumer,
                        frames_repo.clone(),
                        capture_store.clone(),
                        ExclusionEngine::with_extras(
                            cfg_now.exclusion.extra_app_bundle_ids.clone(),
                        ),
                        adapter_registry.clone(),
                        device_id,
                        pipeline_config,
                        foreground_monitor,
                    )))
                }
                Some(Err(error)) => {
                    tracing::warn!(?error, "foreground_monitor.install_failed_degraded_mode");
                    None
                }
                None => {
                    tracing::warn!("capture_pipeline.skipped_degraded_mode: helper unavailable");
                    None
                }
            };

            // Phase 5 hotkey bookkeeping (Batch 5 wires the actual
            // global-shortcut plugin handler).
            let hotkey_service = Arc::new(HotkeyService::new());

            // Chat repos used by the corivo-agent-backed chat path
            // (see commands::exec_agent + commands::quick_ask) and by
            // the thread/message CRUD commands the UI hits directly.
            let chat_threads: Arc<dyn crate::db::repos::chat::ChatThreadRepo> =
                Arc::new(SqliteChatThreadRepo::new(db.pool()));
            let chat_messages: Arc<dyn crate::db::repos::chat::ChatMessageRepo> =
                Arc::new(SqliteChatMessageRepo::new(db.pool()));
            let notes_repo: Arc<dyn crate::db::repos::notes::NotesRepo> =
                Arc::new(SqliteNotesRepo::new(db.pool()));

            // Phase 5 retention sweep — drops frames + screenshots +
            // agent-session jsonl past the 90-day cutoff every hour so
            // the local footprint stays bounded. First round fires after
            // the first sleep so the user doesn't see disk thrash on
            // launch. Spec §8.1 jsonl path is `app_data_dir/corivo-agent-sessions/`.
            let retention_task = Arc::new(RetentionTask::new(
                frames_repo.clone(),
                capture_store.clone(),
                Some(app_data_dir.join("corivo-agent-sessions")),
            ));
            {
                let task = retention_task.clone();
                tauri::async_runtime::spawn(async move {
                    task.start().await;
                });
            }

            // Permission-resolution channel for the corivo-agent path.
            // Lives on AppState independently of the UDS bridge so the
            // agent flow keeps working on platforms where the bridge
            // can't bind (Windows). External corivo-mcp clients, when
            // the bridge IS up, get a clone of the same map so a single
            // `exec_agent_permission_reply` Tauri command resolves
            // requests from both surfaces.
            let permission_pending = crate::services::exec_agent::mcp_bridge::new_pending_map();

            // Long-lived MCP bridge — UDS server the standalone
            // `corivo-mcp` sidecar (used by EXTERNAL clients) dials back
            // into for permission prompts and recall lookups. Boot best-
            // effort: bind failure (or Windows, where AF_UNIX isn't
            // available in tokio) just leaves the field None — the
            // corivo-agent path no longer needs the bridge to be up.
            let exec_agent_bridge =
                match McpBridge::start(app.handle().clone(), permission_pending.clone()) {
                    Ok(bridge) => Some(bridge),
                    Err(error) => {
                        tracing::warn!(?error, "exec_agent.bridge_start_failed");
                        None
                    }
                };

            // Connector framework — Gmail today, future Notion / Slack /
            // Calendar / etc. follow the same `packages/connector-<id>/`
            // shape with no Rust-side per-service code. The registry is
            // always mounted; whether the user has any connectors enabled
            // is runtime state inspected via `connectors_list`.
            let connector_registry = Arc::new(ConnectorRegistry::new(
                app.handle().clone(),
                config_service.clone(),
            ));

            // Phase C §7.5: model catalog cache. The picker UI hits
            // `models_get_available` synchronously; closed builds wire
            // the remote refresh through the private cloud session
            // service.
            let model_catalog = match app.path().app_data_dir() {
                Ok(dir) => Some(Arc::new(crate::services::model_catalog::ModelCatalog::new(
                    dir,
                ))),
                Err(error) => {
                    tracing::warn!(?error, "model_catalog.app_data_dir_unavailable");
                    None
                }
            };
            // Seed the user-editable `Agent.md` / `Tools.md` from the
            // bundled defaults at boot. `load_local_context` is
            // idempotent — it only writes when a file is missing, never
            // overwrites user edits — so calling it eagerly here just
            // pulls the lazy seed forward to first launch instead of
            // first exec_agent turn. Notes are skipped (None) because
            // the persistent block lookup is per-turn anyway.
            if let Ok(dir) = app.path().app_data_dir() {
                let _ = tauri::async_runtime::block_on(load_local_context(
                    &dir,
                    crate::services::exec_agent::local_context::LoadInputs::default(),
                ));
            }

            // Host-skill bridge. Reconcile once at boot so symlinks
            // match `enabled` after a fresh launch (the user may have
            // installed/removed skills on host since last close).
            let skill_share = match app.path().app_data_dir() {
                Ok(dir) => Some(Arc::new(SkillShareService::new(dir))),
                Err(error) => {
                    tracing::warn!(?error, "skill_share.app_data_dir_unavailable");
                    None
                }
            };
            // First-run seed: when `skill_share.initialized == false`
            // (fresh `config.json`), enable every skill currently on
            // host so the corivo-agent has working tooling out of the
            // box. The latch flips to `true` after we persist, so any
            // later "uncheck" the user makes survives subsequent boots.
            if let Some(svc) = skill_share.as_ref() {
                if !cfg_now.exec_agent.skill_share.initialized {
                    let mut seeded = cfg_now.clone();
                    seeded.exec_agent.skill_share.enabled =
                        svc.scan().into_iter().map(|s| s.name).collect();
                    seeded.exec_agent.skill_share.initialized = true;
                    if let Err(error) = config_service.update(seeded) {
                        tracing::warn!(?error, "skill_share.first_run_seed_failed");
                    } else {
                        cfg_now = config_service.get();
                        tracing::info!(
                            count = cfg_now.exec_agent.skill_share.enabled.len(),
                            "skill_share.first_run_seeded"
                        );
                    }
                }
                if let Err(error) = svc.sync(&cfg_now.exec_agent.skill_share.enabled) {
                    tracing::warn!(?error, "skill_share.boot_sync_failed");
                }
            }

            let start_capture_on_launch = cfg_now.app.start_capture_on_launch;

            app.manage(AppLifecycleControl::default());

            // Install the pause-change listener on the pipeline so the
            // menubar tray rebuilds whenever pause/resume flips —
            // including the auto-resume timer firing on its own. Tied
            // to the AppHandle lifetime; pipeline outlives that, so we
            // capture a weak clone of the handle.
            if let Some(pipeline) = capture_pipeline.as_ref() {
                let app_for_cb = app.handle().clone();
                pipeline.set_pause_change_callback(move || {
                    let app_for_task = app_for_cb.clone();
                    tauri::async_runtime::spawn(async move {
                        refresh_tray_menu(&app_for_task).await;
                        if let Err(error) =
                            app_for_task.emit(crate::domain::config::ConfigChanged::EVENT, &())
                        {
                            tracing::warn!(?error, "capture_pipeline.pause_change_emit_failed");
                        }
                    });
                });
            }

            // Background agent task scheduler — memory-system-spec §11.
            // The worker loop runs forever; session learner idle hook
            // enqueues threads it sees idle on a fixed tick.
            let bg_scheduler = Arc::new(BackgroundAgentScheduler::new(app.handle().clone()));
            bg_scheduler.start();

            // Scheduled-workflows store + ticker (v1430). Lives on
            // top of the BackgroundAgentScheduler — the Ticker resolves
            // due `workflow_schedules` rows, advances `next_run_at`,
            // and enqueues a `ScheduledWorkflowTask` per dispatch.
            let workflow_store =
                WorkflowStore::with_app(db.pool(), &app_data_dir, app.handle().clone());
            if let Err(error) = workflow_store.ensure_root() {
                tracing::warn!(?error, "scheduled_workflow.root_create_failed");
            }
            // First-boot preset seed. Idempotent + best-effort: a
            // failure here just means daily-review is missing from
            // the empty state; the user can create it by hand.
            services::scheduled_workflows::presets::seed_defaults(&workflow_store);
            let workflow_ticker = ScheduledWorkflowTicker::new(
                workflow_store.clone(),
                (*bg_scheduler).clone(),
            );
            workflow_ticker.clone().start();

            let idle_hook = services::session_learner::IdleHook::new(
                app.handle().clone(),
                (*bg_scheduler).clone(),
                std::time::Duration::from_secs(25 * 60),
                std::time::Duration::from_secs(60),
            );
            idle_hook.start();

            // Persona distill daily refresh (memory-system-spec §4.3).
            // Runs once at boot if the existing auto-persona.md is
            // missing/stale (>24h), then once per day.
            services::persona::scheduler::start(app.handle().clone(), (*bg_scheduler).clone());

            // Cloud capability bundle. Open-source default: every field
            // is a noop trait impl that returns `FeatureUnavailable`,
            // and the frontend hides the corresponding UI via the
            // `get_capabilities` probe. Forks plugging in a cloud
            // backend should mutate `services` between this line and
            // the `Arc::new` wrap below.
            let cloud = crate::services::cloud::CloudServices::noop();
            #[cfg(feature = "corivo-cloud")]
            let cloud = crate::services::cloud::private_hooks::build_cloud_services(
                cloud,
                app.handle().clone(),
                config_service.clone(),
                model_catalog.clone(),
                connector_registry.clone(),
                &cfg_now,
            );
            let cloud = std::sync::Arc::new(cloud);

            // Privacy filter (docs/privacy-filter-spec.md): bootstrap from
            // persisted Config. 默认 enabled=false —— 用户必须在 Settings
            // 同意下载模型后才打开。secret 类目永远强制 on(spec §12.1)。
            // 模型 session/decode/redact 走 services::privacy_filter,这
            // 里只挂句柄,不加载任何文件。
            let privacy_filter = std::sync::Arc::new(services::privacy_filter::PrivacyFilter::new(
                config_service.get().privacy_filter.clone(),
            ));

            app.manage(AppState {
                db,
                capture_store,
                config_service: config_service.clone(),
                capture_pipeline: capture_pipeline.clone(),
                frames_repo: Some(frames_repo),
                chat_threads: Some(chat_threads),
                chat_messages: Some(chat_messages.clone()),
                notes_repo: Some(notes_repo),
                hotkey_service: Some(hotkey_service.clone()),
                exec_agent_bridge,
                permission_pending,
                skill_share,
                capture_client,
                model_catalog,
                connector_registry: Some(connector_registry),
                bg_scheduler: Some(bg_scheduler),
                workflow_store: Some(workflow_store),
                workflow_ticker: Some(workflow_ticker),
                pending_turns: std::sync::Arc::new(tokio::sync::Mutex::new(
                    std::collections::HashMap::new(),
                )),
                cloud,
                privacy_filter,
            });

            // Restore a persisted menubar pause if the expiry is still
            // in the future. Done synchronously (block_on) so the
            // subsequent menu paint + auto-start branch both see
            // consistent state. Past expiry → wipe the stale value so
            // the next pause writes a fresh one.
            if let Some(pipeline) = capture_pipeline.as_ref() {
                let persisted = cfg_now.app.capture_paused_until;
                let now = chrono::Utc::now();
                match persisted {
                    Some(until) if until > now => {
                        let pipeline_for_restore = pipeline.clone();
                        if let Err(error) = tauri::async_runtime::block_on(async move {
                            pipeline_for_restore.pause_until(until).await
                        }) {
                            tracing::warn!(?error, "capture_pipeline.pause_restore_failed");
                        }
                    }
                    Some(_) => {
                        let mut cleared = cfg_now.clone();
                        cleared.app.capture_paused_until = None;
                        if let Err(error) = config_service.update(cleared) {
                            tracing::warn!(?error, "capture_pipeline.pause_stale_clear_failed");
                        }
                    }
                    None => {}
                }
            }

            setup_menu_bar_preview(app.handle())?;

            // v1200 boot hook: any chat_messages row left in
            // `status='streaming'` from a prior crash / kill is flipped
            // to `cancelled`. Without this the UI sees a perpetually-
            // spinning placeholder. Best-effort: failures log + carry on.
            {
                let messages = chat_messages.clone();
                tauri::async_runtime::spawn(async move {
                    match messages.cancel_streaming_orphans().await {
                        Ok(0) => {}
                        Ok(n) => tracing::info!(count = n, "chat.cancel_streaming_orphans done"),
                        Err(error) => {
                            tracing::warn!(?error, "chat.cancel_streaming_orphans failed")
                        }
                    }
                });
            }

            apply_quick_ask_vibrancy(app.handle());
            if let Some(window) = app.get_webview_window(QUICK_ASK_WINDOW_LABEL) {
                apply_quick_ask_overlay_window_mode(&window);
            }

            // Quick Ask is summoned by double-tapping ⌥ (Option). We
            // install the NSEvent global monitor *after* AppState is
            // mounted because the on-fire callback hops through
            // `on_quick_ask_hotkey`, which reads `capture_pipeline` /
            // window state from `AppState` — installing earlier would
            // race against an empty state on the first press.
            //
            // `RegisterEventHotKey` (the Carbon API behind
            // `tauri-plugin-global-shortcut`) can't bind bare modifiers,
            // so we don't go through that plugin at all anymore.
            let app_for_hotkey = app.handle().clone();
            match DoubleTapHotkey::install(move || {
                on_quick_ask_hotkey(&app_for_hotkey);
            }) {
                Ok(monitor) => {
                    // Tauri-managed singleton — kept alive for the
                    // process lifetime; Drop unregisters the NSEvent
                    // monitor.
                    app.manage(monitor);
                    let svc = hotkey_service.clone();
                    tauri::async_runtime::spawn(async move {
                        svc.record_installed().await;
                    });
                }
                Err(error) => {
                    tracing::warn!(?error, "double_tap_hotkey.install_failed");
                    let svc = hotkey_service.clone();
                    let err_str = error.to_string();
                    tauri::async_runtime::spawn(async move {
                        svc.record_install_failure(err_str).await;
                    });
                }
            }

            // Cmd+Shift+O — global "summon main window". Goes through
            // `tauri-plugin-global-shortcut` (RegisterEventHotKey under
            // the hood) because, unlike Quick Ask's bare ⌥, this chord
            // has a real key code attached and is exactly what that API
            // is designed for. `show_main_window` is dispatched via
            // `run_on_main_thread` because it touches AppKit window ops
            // (see CLAUDE.md threading note + `on_quick_ask_hotkey`).
            #[cfg(desktop)]
            {
                use tauri_plugin_global_shortcut::{
                    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
                };
                let summon = Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyO);
                if let Err(error) =
                    app.global_shortcut()
                        .on_shortcut(summon, |app, _shortcut, event| {
                            if event.state() != ShortcutState::Pressed {
                                return;
                            }
                            let handle = app.clone();
                            if let Err(error) = handle.clone().run_on_main_thread(move || {
                                show_main_window(&handle);
                            }) {
                                tracing::warn!(?error, "summon_main.dispatch_failed");
                            }
                        })
                {
                    tracing::warn!(?error, "summon_main.shortcut.register_failed");
                } else {
                    tracing::info!("summon_main.shortcut.registered (cmd+shift+o)");
                }
            }

            if start_capture_on_launch {
                if let Some(pipeline) = capture_pipeline.clone() {
                    tauri::async_runtime::spawn(async move {
                        if pipeline.is_running() {
                            return;
                        }
                        // Restored menubar pause keeps capture off
                        // until the timer expires (or the user
                        // resumes) — auto-start must not punch through.
                        if pipeline.is_paused() {
                            tracing::info!("capture_pipeline.auto_start_skipped_paused");
                            return;
                        }
                        if let Err(error) = pipeline.start().await {
                            tracing::error!(?error, "capture_pipeline.auto_start_failed");
                        }
                    });
                } else {
                    tracing::warn!("capture_pipeline.auto_start_skipped_degraded_mode");
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != MAIN_WINDOW_LABEL {
                    return;
                }

                let app_handle = window.app_handle();
                let config = if let Some(state) = app_handle.try_state::<AppState>() {
                    state.config_service.get()
                } else {
                    return;
                };

                match resolve_close_request_action(&config, is_app_quitting(app_handle)) {
                    CloseRequestAction::HideToTray => {
                        api.prevent_close();
                        let _ = window.hide();
                        let _ = sync_macos_system_surface_from_runtime(
                            app_handle,
                            MAIN_WINDOW_LABEL,
                            false,
                            false,
                            false,
                        );
                    }
                    CloseRequestAction::Shutdown => {
                        if !is_app_quitting(app_handle) {
                            api.prevent_close();
                            request_app_quit(app_handle);
                        } else {
                            shutdown_runtime_once(app_handle);
                        }
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::config::get_config,
            commands::config::set_config,
            // 构建态 cloud-capability 探针 — 前端 boot 时拉一次决定路由
            // 和 UI 分支。开源构建里所有字段都是 false。
            commands::cloud::get_capabilities,
            #[cfg(feature = "corivo-cloud")]
            commands::composio::composio_list_connections,
            #[cfg(feature = "corivo-cloud")]
            commands::composio::composio_create_connection_link,
            #[cfg(feature = "corivo-cloud")]
            commands::composio::composio_disconnect,
            commands::privacy::get_privacy_settings,
            commands::privacy::set_privacy_settings,
            commands::privacy::clear_privacy_cache,
            commands::privacy::get_privacy_model_status,
            commands::privacy::download_privacy_model,
            commands::privacy::delete_privacy_model,
            commands::onboarding::get_onboarding_state,
            commands::onboarding::save_onboarding_step,
            commands::onboarding::mark_onboarding_completed,
            commands::onboarding::mark_first_quick_ask_done,
            commands::onboarding::reset_onboarding,
            commands::settings::set_autostart_enabled,
            commands::settings::get_autostart_enabled,
            commands::settings::get_system_info,
            commands::settings::clear_all_screenshots,
            commands::settings::data_hard_delete,
            commands::settings::data_delete_range,
            commands::settings::check_screen_recording_permission,
            commands::settings::request_screen_recording_permission,
            commands::settings::open_system_settings_privacy,
            commands::settings::check_ax_permission,
            commands::settings::open_ax_settings,
            commands::settings::open_captures_directory,
            commands::settings::get_data_paths,
            // v3 capture + frames surface (spec §十一)
            commands::capture::capture_status,
            commands::capture::capture_start,
            commands::capture::capture_stop,
            commands::capture::capture_pause,
            commands::capture::capture_resume,
            commands::capture::capture_request_snapshot,
            commands::capture::capture_diagnostic_status,
            commands::capture::exclusion_list,
            commands::capture::exclusion_add,
            commands::capture::exclusion_remove,
            commands::capture::website_exclusion_list,
            commands::capture::website_exclusion_add,
            commands::capture::website_exclusion_remove,
            commands::frames::frames_list,
            commands::frames::frame_detail,
            // /ask (spec §十一)
            commands::chat::chat_threads_list,
            commands::chat::chat_thread_create,
            commands::chat::chat_thread_delete,
            commands::chat::chat_thread_set_pinned,
            commands::chat::chat_thread_set_archived,
            commands::chat::chat_messages_by_thread,
            // v1200 three-step turn lifecycle (replaces chat_persist_turn).
            // See commands::chat module doc-comment for the ordering.
            commands::chat::chat_user_message_create,
            commands::chat::chat_assistant_message_start,
            commands::chat::chat_assistant_message_finalize,
            // Closed-beta auth (Google OAuth loopback → bearer token in config.json)
            commands::auth::auth_login_google,
            commands::auth::auth_request_email_code,
            commands::auth::auth_login_email,
            commands::auth::auth_logout,
            commands::auth::auth_status,
            commands::auth::auth_refresh,
            // Connector framework (Settings → Integrations)
            commands::connectors::connectors_list,
            commands::connectors::connector_enable,
            commands::connectors::connector_disable,
            commands::connectors::connector_connect,
            commands::connectors::connector_provider_connect,
            commands::connectors::connector_install_mcp,
            commands::connectors::connector_disconnect,
            // Stripe top-up flow (BillingDialog)
            commands::billing::billing_start_checkout,
            commands::billing::billing_me,
            // Skill-share Settings UI (read-only listing — mutation goes
            // through set_config + skill_share.sync hook)
            commands::skills::skills_list_available,
            // Phase C §7.5 model catalog (Settings model picker)
            commands::models::models_get_available,
            commands::models::models_refresh,
            commands::models::models_set_active_model,
            // Executive agent (spawn `corivo-agent` sidecar)
            commands::exec_agent::exec_agent_send,
            commands::exec_agent::exec_agent_permission_reply,
            commands::exec_agent::exec_agent_cancel,
            // Quick Ask (spec §十一) — the send turn itself goes through
            // `exec_agent_send` with a focus_context payload; this module
            // only owns hotkey + window + capture + open-in-app.
            commands::quick_ask::quick_ask_summon,
            commands::quick_ask::quick_ask_hide,
            commands::quick_ask::quick_ask_set_height,
            commands::quick_ask::quick_ask_log_display,
            commands::quick_ask::quick_ask_capture_focus,
            commands::quick_ask::quick_ask_open_in_app,
            commands::quick_ask::hotkey_status,
            // App-icon lookup for Quick Ask FocusCard (NSWorkspace
            // iconForFile: → 32×32 PNG → base64 data URL).
            commands::app_icon::get_app_icon,
            // Frontend → tracing log bridge (used by updater UI etc.)
            commands::log::log_frontend,
            // 业务埋点 → Sentry message event（独立可见，不依赖 error）
            commands::analytics::track_event,
            // Memory system (memory-system-spec §10.5) — notes CRUD for
            // the Settings UI and (later) the recall debug surface.
            commands::memory::list_notes,
            commands::memory::get_note,
            commands::memory::create_note,
            commands::memory::update_note,
            commands::memory::delete_note,
            commands::memory::list_system_threads,
            commands::memory::rerun_session_learner,
            commands::memory::get_auto_persona,
            commands::memory::regenerate_auto_persona,
            commands::memory::open_background_task_logs_dir,
            // Scheduled workflows (v1430) — /workflows page IPC surface.
            commands::workflows::workflows_list,
            commands::workflows::workflows_list_runs,
            commands::workflows::workflows_get_run,
            commands::workflows::workflows_preview_trigger,
            commands::workflows::workflows_save,
            commands::workflows::workflows_delete,
            commands::workflows::workflows_set_enabled,
            commands::workflows::workflows_run_now,
            commands::workflows::workflows_cancel_run,
            commands::workflows::workflows_acknowledge_run,
            commands::workflows::workflows_unread_count,
            // Hidden developer-mode tools (unlocked by triple-clicking
            // the Settings dialog title; surfaced in the "Developer" tab).
            commands::dev::dev_open_devtools,
            commands::dev::dev_reveal_data_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use chrono::NaiveDate;
    use tempfile::TempDir;

    use crate::domain::config::Config;

    use super::{
        dated_log_file_path, ensure_log_directory, init_tracing, parse_tray_menu_action,
        resolve_close_request_action, CloseRequestAction, TrayMenuAction,
    };

    #[test]
    fn tracing_init_is_idempotent() {
        let temp_dir = TempDir::new().expect("temp dir");
        init_tracing(temp_dir.path());
        init_tracing(temp_dir.path());
    }

    #[test]
    fn ensure_log_directory_creates_logs_subdirectory_under_app_data_dir() {
        let temp_dir = TempDir::new().expect("temp dir");
        let log_dir = ensure_log_directory(temp_dir.path()).expect("log dir");
        assert_eq!(log_dir, temp_dir.path().join("logs"));
        assert!(Path::new(&log_dir).is_dir());
    }

    #[test]
    fn dated_log_file_path_uses_yyyy_mm_dd_log_filename() {
        let log_dir = Path::new("/tmp/corivo-logs");
        let date = NaiveDate::from_ymd_opt(2026, 4, 15).expect("valid date");
        let path = dated_log_file_path(log_dir, date);
        assert_eq!(path, log_dir.join("2026-04-15.log"));
    }

    #[test]
    fn close_request_action_hides_main_window_when_minimize_to_tray_is_enabled() {
        let config = Config::default();
        let action = resolve_close_request_action(&config, false);
        assert_eq!(action, CloseRequestAction::HideToTray);
    }

    #[test]
    fn close_request_action_allows_shutdown_when_minimize_to_tray_is_disabled() {
        let mut config = Config::default();
        config.app.minimize_to_tray = false;
        let action = resolve_close_request_action(&config, false);
        assert_eq!(action, CloseRequestAction::Shutdown);
    }

    #[test]
    fn close_request_action_allows_shutdown_when_app_is_already_quitting() {
        let config = Config::default();
        let action = resolve_close_request_action(&config, true);
        assert_eq!(action, CloseRequestAction::Shutdown);
    }

    #[test]
    fn tray_menu_action_maps_open_and_quit_ids() {
        assert_eq!(
            parse_tray_menu_action("tray-open"),
            Some(TrayMenuAction::OpenMainWindow)
        );
        assert_eq!(
            parse_tray_menu_action("tray-quit"),
            Some(TrayMenuAction::QuitApp)
        );
    }

    #[test]
    fn tray_menu_action_maps_pause_and_resume_ids() {
        assert_eq!(
            parse_tray_menu_action("tray-pause-hour"),
            Some(TrayMenuAction::PauseHour)
        );
        assert_eq!(
            parse_tray_menu_action("tray-pause-day"),
            Some(TrayMenuAction::PauseDay)
        );
        assert_eq!(
            parse_tray_menu_action("tray-resume"),
            Some(TrayMenuAction::Resume)
        );
    }

    #[test]
    fn tray_menu_action_ignores_unknown_ids() {
        assert_eq!(parse_tray_menu_action("tray-unknown"), None);
    }
}
