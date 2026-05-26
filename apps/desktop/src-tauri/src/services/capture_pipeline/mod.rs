//! Event-driven capture pipeline.
//!
//! Each frame is the result of an event arriving on
//! [`event_bus::channel`]. Sources fan in (NSWorkspace front-app
//! shifts, AXObserver notifications on the active app, manual IPC,
//! safety-net cadence) → [`coordinator`] debounces per-surface →
//! [`coordinator::run_jobs`] worker runs `tick()` for each emitted
//! [`debouncer::SnapshotJob`].
//!
//! Each `tick()`:
//!
//! 1. Skip if system has been idle past `idle_skip_secs`.
//! 2. Probe the foreground app via [`foreground::probe`].
//! 3. Run the exclusion engine on the bundle id. On a hit, write a
//!    `Skipped` envelope (no screenshot, no extractor).
//! 4. Dedup ladder ([`dedup`]) — metadata-only. On match, bump
//!    `still_present_until` on the last frame and skip the rest.
//! 5. Run extractor ([`extractor::extract`]). The extractor takes a
//!    screenshot **on demand** if AX comes back empty and OCR fallback
//!    is enabled; otherwise no image is taken.
//! 6. Build [`SnapshotEnvelope`] and hand it to the consumer. The
//!    `screenshot` field carries whatever OCR fallback persisted, or
//!    `None` if AX produced the body.

pub mod coordinator;
pub mod debouncer;
pub mod dedup;
pub mod event_bus;
pub mod event_sources;
pub mod foreground;
pub mod screen_capture;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tokio::{
    sync::{mpsc, Mutex},
    task::JoinHandle,
};

use crate::{
    db::repos::frames::FrameRepo,
    domain::snapshot_envelope::{
        AppInfo, ExtractionResult, ExtractionStrategy, FrameSnapshot, RawDebug, SnapshotEnvelope,
        Trigger, WindowInfo, ENVELOPE_VERSION,
    },
    error::{CorivoError, Result},
    services::{
        capture_store::CaptureStore,
        exclusion::{ExclusionEngine, ExclusionVerdict},
        extractor::{self, ExtractInput, OcrResources},
        foreground_monitor::ForegroundAppMonitor,
        snapshot_consumer::SnapshotConsumer,
    },
};

use coordinator::{spawn as spawn_coordinator, JobRunnerArgs};
use dedup::{classify_metadata, CurrentMetadata, DedupDecision};
use event_bus::Event;
use event_sources::AXObserverThread;

#[derive(Debug, Clone)]
pub struct CapturePipelineConfig {
    /// Safety-net cadence in seconds. The pipeline is event-driven —
    /// this only paces the periodic "I'm still here" tick that catches
    /// missed observers. Real captures are driven by NSWorkspace +
    /// AXObserver events.
    pub interval_secs: u64,
    /// JPEG quality 0-100. Default 75.
    pub jpeg_quality: u8,
    /// Idle threshold in seconds. If the system has been idle this long,
    /// safety-net ticks skip and observer-driven ticks still capture
    /// (the user's intent, not idleness, decided to fire).
    pub idle_skip_secs: u64,
}

impl Default for CapturePipelineConfig {
    fn default() -> Self {
        Self {
            interval_secs: 60,
            jpeg_quality: 75,
            idle_skip_secs: 60,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapturePhase {
    Stopped,
    Running,
}

impl CapturePhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Running => "running",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CaptureStatus {
    pub phase: CapturePhase,
    pub current_session_id: Option<String>,
    /// `Some(until)` when the user has paused screen reading from the
    /// menubar tray. Pause is independent of `phase` — it short-circuits
    /// `start()` and blocks the Quick Ask hotkey path. Cleared either by
    /// an explicit Resume or by the auto-resume timer reaching `until`.
    pub paused_until: Option<chrono::DateTime<chrono::Utc>>,
}

/// Internal pause bookkeeping. Held under
/// `CapturePipeline::pause_state` while the user has the pipeline
/// paused from the menubar; the `was_running` flag drives whether
/// auto-resume re-spawns the event loop on expiry.
#[derive(Debug, Clone)]
pub struct PauseState {
    pub paused_until: chrono::DateTime<chrono::Utc>,
    pub was_running: bool,
}

/// Phase-A output for [`CapturePipeline::invoke_quick_ask_phase_a`].
///
/// `Excluded` short-circuits the two-phase split: the exclusion path
/// has no extraction work, so we already wrote the skipped-frame and
/// have a complete [`FocusContext`] to emit. `Pending` carries the
/// public skeleton (FocusCard renders it immediately) plus the
/// internal state phase B needs to finish the work.
pub enum QuickAskPhaseAOutcome {
    Excluded(crate::domain::focus_context::FocusContext),
    Pending {
        skeleton: crate::domain::focus_context::QuickAskSkeleton,
        state: QuickAskPendingState,
    },
}

/// Internal handoff between phase A and phase B. Opaque to callers —
/// the hotkey flow keeps it on the stack until phase B is spawned.
pub struct QuickAskPendingState {
    probe: foreground::ForegroundProbe,
    session_id: String,
}

pub struct CapturePipeline {
    config: Arc<Mutex<CapturePipelineConfig>>,
    consumer: Arc<dyn SnapshotConsumer>,
    frames: Arc<dyn FrameRepo>,
    capture_store: Arc<CaptureStore>,
    exclusion: Arc<Mutex<ExclusionEngine>>,
    /// Per-app adapter registry shared across all extraction calls.
    /// Phase 5: see services/extractor/adapters/registry.rs.
    adapters: Arc<crate::services::extractor::AdapterRegistry>,
    device_id: String,
    running: Arc<AtomicBool>,
    /// Coordinator + job runner JoinHandles spawned in `start()`.
    event_workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
    /// Event-bus sender retained on the pipeline so manual triggers
    /// (`request_snapshot`) can publish without re-creating the bus.
    /// Cleared on `stop`.
    event_tx: Arc<Mutex<Option<mpsc::Sender<Event>>>>,
    session_id: Arc<Mutex<Option<String>>>,
    /// Dedicated CFRunLoop thread driving AXObserver subscriptions
    /// against the current foreground app. `None` while stopped.
    ax_observer: Arc<Mutex<Option<AXObserverThread>>>,
    /// Quick Ask invokes happen outside the main `start`/`stop` flow
    /// (a Tauri command races with the run loop), so we keep a
    /// `quick_ask_session_id` that survives stop/restart of capture.
    /// In practice it mirrors the active session id when capture is
    /// running, and falls back to a synthetic "quick-ask-only" session
    /// id when the user invokes Quick Ask before starting capture.
    quick_ask_session: Arc<Mutex<Option<String>>>,
    /// Frontmost-app monitor. Lives for the whole process; we
    /// `subscribe` to it on `start` and `unsubscribe` on `stop`. The
    /// monitor itself keeps emitting `capture:focus-activated` to the
    /// Quick Ask overlay regardless of capture state.
    foreground_monitor: Arc<ForegroundAppMonitor>,
    /// Pause bookkeeping (menubar "Pause 1 hour / 1 day"). When set,
    /// every capture entry point (`start`, `request_snapshot`,
    /// `invoke_quick_ask_phase_a`) bails out — screen reading is fully
    /// frozen until either the auto-resume timer fires or the user
    /// flips Resume from the tray.
    pause_state: Arc<Mutex<Option<PauseState>>>,
    /// Background tokio task that flips pause off when `paused_until`
    /// is reached. Aborted on explicit `resume()` so manual resume
    /// doesn't race the timer.
    pause_timer: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Fast, lock-free read for hot paths (Quick Ask hotkey, capture
    /// loop entry). Kept in sync with `pause_state.is_some()`.
    pause_active: Arc<AtomicBool>,
    /// Optional listener fired after every pause/resume mutation
    /// (including the auto-resume timer firing on its own). Set once
    /// at boot from `lib.rs` so the menubar tray can rebuild its menu
    /// without polling. Stored on `Mutex` because we only ever read it
    /// — registration happens once and write contention is nil.
    pause_change_cb: Arc<std::sync::Mutex<Option<Box<dyn Fn() + Send + Sync + 'static>>>>,
}

impl CapturePipeline {
    pub fn new(
        consumer: Arc<dyn SnapshotConsumer>,
        frames: Arc<dyn FrameRepo>,
        capture_store: Arc<CaptureStore>,
        exclusion: ExclusionEngine,
        adapters: Arc<crate::services::extractor::AdapterRegistry>,
        device_id: String,
        config: CapturePipelineConfig,
        foreground_monitor: Arc<ForegroundAppMonitor>,
    ) -> Self {
        Self {
            config: Arc::new(Mutex::new(config)),
            consumer,
            frames,
            capture_store,
            exclusion: Arc::new(Mutex::new(exclusion)),
            adapters,
            device_id,
            running: Arc::new(AtomicBool::new(false)),
            event_workers: Arc::new(Mutex::new(Vec::new())),
            event_tx: Arc::new(Mutex::new(None)),
            session_id: Arc::new(Mutex::new(None)),
            ax_observer: Arc::new(Mutex::new(None)),
            quick_ask_session: Arc::new(Mutex::new(None)),
            foreground_monitor,
            pause_state: Arc::new(Mutex::new(None)),
            pause_timer: Arc::new(Mutex::new(None)),
            pause_active: Arc::new(AtomicBool::new(false)),
            pause_change_cb: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Install a one-shot listener fired after every pause-state
    /// mutation (manual pause/resume, auto-resume timer, …). Used by
    /// `lib.rs` to rebuild the menubar tray menu without polling.
    /// Calling a second time replaces the previous listener.
    pub fn set_pause_change_callback<F>(&self, cb: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        *self.pause_change_cb.lock().unwrap() = Some(Box::new(cb));
    }

    fn notify_pause_change(&self) {
        if let Some(cb) = self.pause_change_cb.lock().unwrap().as_ref() {
            cb();
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Fast lock-free read for hot paths (Quick Ask hotkey, capture
    /// loop entry, IPC pause gating). Source of truth is
    /// `pause_state.lock().await` — keep the two in sync at every
    /// mutation site.
    pub fn is_paused(&self) -> bool {
        self.pause_active.load(Ordering::SeqCst)
    }

    pub async fn paused_until(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.pause_state
            .lock()
            .await
            .as_ref()
            .map(|p| p.paused_until)
    }

    pub async fn status(&self) -> CaptureStatus {
        CaptureStatus {
            phase: if self.is_running() {
                CapturePhase::Running
            } else {
                CapturePhase::Stopped
            },
            current_session_id: self.session_id.lock().await.clone(),
            paused_until: self.paused_until().await,
        }
    }

    pub async fn update_config(&self, mut new: CapturePipelineConfig) {
        new.interval_secs = new.interval_secs.max(1);
        *self.config.lock().await = new;
    }

    /// Replace the active exclusion engine. Called from
    /// `commands::capture::exclusion_*` after the user edits the list
    /// in Settings — the pipeline picks up the new engine on its next
    /// tick without needing a stop/restart.
    pub async fn update_exclusion(&self, engine: ExclusionEngine) {
        *self.exclusion.lock().await = engine;
    }

    /// Boot the event-driven capture loop. Wires:
    ///   1. [`event_bus::channel`] — single mpsc all sources fan into.
    ///   2. [`AXObserverThread`] — dedicated CFRunLoop thread that
    ///      subscribes to AX notifications on the current pid.
    ///   3. Subscribe to the long-lived
    ///      [`ForegroundAppMonitor`] so each NSWorkspace
    ///      `AppActivated` pushes onto the bus and retargets the AX
    ///      thread to the new pid.
    ///   4. [`coordinator::spawn`] debounces events into
    ///      [`debouncer::SnapshotJob`]s.
    ///   5. [`coordinator::run_jobs`] worker runs `tick()` per job.
    ///
    /// **No timer fallback.** If observers stop firing (AX permission
    /// not granted, Mac sleeping, etc.) capture goes silent. That's
    /// the design — the alternative is a fixed-cadence safety net
    /// that masks the real failure (which is what the previous
    /// SafetyNet did, and why it felt like a 5s timer when AX wasn't
    /// installed).
    pub async fn start(&self) -> Result<String> {
        // Pause short-circuits start so an explicit user pause survives
        // both auto-start-on-launch and any UI mash on the sidebar
        // RESUME row.
        if self.is_paused() {
            return Err(CorivoError::Internal(
                "CapturePipeline is paused; resume from the menubar first".to_string(),
            ));
        }

        if self.running.swap(true, Ordering::SeqCst) {
            return Err(CorivoError::Internal(
                "CapturePipeline is already running".to_string(),
            ));
        }

        let cfg_now = self.config.lock().await.clone();
        // CaptureStore still wants a `batch_size` arg from the v2 era; pass
        // 1 (each tick is independent). Worth removing when we clean up
        // CaptureStore's API.
        let session = self
            .capture_store
            .create_session(cfg_now.interval_secs, 1)
            .await?;
        let session_id = session.id.clone();
        *self.session_id.lock().await = Some(session_id.clone());

        // Loud, single-line diagnostic. Without AX permission the pipeline
        // is still useful (focus_change events fire) but title_changed and
        // focused_window_changed go silent — making capture feel sparse.
        let ax_trusted = crate::services::extractor::ax_extractor::is_process_trusted().await;
        if !ax_trusted {
            tracing::warn!(
                "capture_pipeline.ax_not_trusted — title_changed / focused_window_changed \
                 events will not fire. Grant Accessibility in System Settings → Privacy & \
                 Security → Accessibility, then restart capture."
            );
        }

        tracing::info!(
            session_id = %session_id,
            jpeg_quality = cfg_now.jpeg_quality,
            ax_trusted,
            "capture_pipeline.session_started"
        );

        let (event_tx, event_rx) = event_bus::channel();
        *self.event_tx.lock().await = Some(event_tx.clone());

        // Source: AXObserver thread. Spawned before subscribing to the
        // foreground monitor so we can hand it a target pid as soon
        // as the first AppActivated fires (or as soon as the
        // initial-focus seed below).
        let ax_observer = AXObserverThread::spawn(event_tx.clone());
        // Seed initial pid from current foreground (the foreground
        // monitor may not fire until the user actually switches apps,
        // and we want AX coverage on the app that was front when
        // capture started).
        if let Some(pid) = foreground::probe_current().await.pid {
            ax_observer.set_target_pid(pid);
        }

        // Subscribe to the long-lived foreground monitor: each
        // NSWorkspace `AppActivated` will now (a) push onto our event
        // bus and (b) retarget the AXObserver thread via the
        // pid_setter closure.
        let pid_setter = ax_observer.pid_setter();
        self.foreground_monitor
            .subscribe(event_tx.clone(), Box::new(move |pid: i32| pid_setter(pid)));

        *self.ax_observer.lock().await = Some(ax_observer);

        // Push a synthetic initial-focus event so the first frame
        // shows up without waiting for an app switch.
        if let Err(error) = ForegroundAppMonitor::emit_initial_focus(&event_tx) {
            tracing::warn!(?error, "capture_pipeline.initial_focus_failed");
        }

        let (job_rx, coordinator_handle) = spawn_coordinator(event_rx);
        let job_runner_handle = tokio::spawn(coordinator::run_jobs(JobRunnerArgs {
            job_rx,
            consumer: self.consumer.clone(),
            frames: self.frames.clone(),
            capture_store: self.capture_store.clone(),
            exclusion: self.exclusion.clone(),
            adapters: self.adapters.clone(),
            config: self.config.clone(),
            running: self.running.clone(),
            device_id: self.device_id.clone(),
            session_id: session_id.clone(),
        }));

        let mut guard = self.event_workers.lock().await;
        guard.push(coordinator_handle);
        guard.push(job_runner_handle);

        Ok(session_id)
    }

    /// Publish a manual snapshot request on the event bus. Returns
    /// `Err` if the pipeline isn't running in event mode (no bus to
    /// publish to). Coordinator may still drop the job under
    /// debounce / exclusion, so this is best-effort.
    pub async fn request_snapshot(&self, reason: impl Into<String>) -> Result<()> {
        if self.is_paused() {
            return Err(CorivoError::Internal(
                "CapturePipeline is paused; manual snapshot ignored".into(),
            ));
        }
        let reason = reason.into();
        let guard = self.event_tx.lock().await;
        let Some(tx) = guard.as_ref() else {
            return Err(CorivoError::Internal(
                "manual snapshot requested but event driver is not running".into(),
            ));
        };
        tx.send(Event::ManualSnapshot { reason })
            .await
            .map_err(|error| CorivoError::Internal(format!("event_bus closed: {error}")))?;
        Ok(())
    }

    /// Forget the cached Quick Ask session id so the next invoke
    /// lazy-creates a fresh one. Used by the "清空所有数据" settings
    /// action after wiping the on-disk session index — otherwise the
    /// stale id would point at a session directory that no longer
    /// exists.
    pub async fn reset_quick_ask_cache(&self) {
        *self.quick_ask_session.lock().await = None;
    }

    pub async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);

        // Detach our capture-side sink from the foreground monitor.
        // The monitor itself stays alive and keeps driving the Quick
        // Ask overlay.
        self.foreground_monitor.unsubscribe();

        // Cooperatively shut down the AX runloop thread.
        if let Some(ax) = self.ax_observer.lock().await.take() {
            ax.shutdown();
        }

        // Send a cooperative Shutdown then drop the sender so the
        // coordinator's bus closes after it drains. abort() backs us
        // up if the coordinator is wedged.
        if let Some(tx) = self.event_tx.lock().await.take() {
            let _ = tx.send(Event::Shutdown).await;
            drop(tx);
        }

        let mut event_workers = self.event_workers.lock().await;
        for handle in event_workers.drain(..) {
            handle.abort();
        }
        drop(event_workers);

        if let Some(session_id) = self.session_id.lock().await.take() {
            self.capture_store.end_session(&session_id, false).await?;
            tracing::info!(session_id = %session_id, "capture_pipeline.session_stopped");
        }
        Ok(())
    }

    /// Pause screen reading for `duration`, persisting `paused_until`
    /// so a restart inside the pause window restores the same state.
    /// Stops the capture pipeline if it's running and remembers that
    /// fact so auto-resume can re-start it. Quick Ask is also blocked
    /// for the duration — see [`Self::invoke_quick_ask_phase_a`].
    ///
    /// `until` is the wall-clock instant the pause expires. A
    /// background tokio task sleeps until then and flips the state
    /// back via `resume_internal`; manual Resume aborts it.
    ///
    /// Re-pausing while already paused replaces the existing timer
    /// rather than stacking them.
    pub async fn pause_until(self: &Arc<Self>, until: chrono::DateTime<chrono::Utc>) -> Result<()> {
        let was_running = self.is_running();
        if was_running {
            self.stop().await?;
        }

        if let Some(existing) = self.pause_timer.lock().await.take() {
            existing.abort();
        }

        *self.pause_state.lock().await = Some(PauseState {
            paused_until: until,
            was_running,
        });
        self.pause_active.store(true, Ordering::SeqCst);

        let now = chrono::Utc::now();
        let sleep_for = until
            .signed_duration_since(now)
            .to_std()
            .unwrap_or_default();

        let pipeline_for_timer = Arc::clone(self);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(sleep_for).await;
            tracing::info!("capture_pipeline.pause_auto_resume");
            if let Err(error) = pipeline_for_timer.resume_internal(true).await {
                tracing::warn!(?error, "capture_pipeline.pause_auto_resume_failed");
            }
        });
        *self.pause_timer.lock().await = Some(handle);

        tracing::info!(
            paused_until = %until.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            was_running,
            "capture_pipeline.paused"
        );
        self.notify_pause_change();
        Ok(())
    }

    /// Explicit Resume from the menubar or IPC. Cancels the timer
    /// and re-starts the pipeline if it was running when pause began.
    pub async fn resume(self: &Arc<Self>) -> Result<()> {
        self.resume_internal(false).await
    }

    /// Shared body for manual + auto resume. `auto = true` is only
    /// passed by the timer task so we can distinguish the two paths
    /// in logs.
    async fn resume_internal(self: &Arc<Self>, auto: bool) -> Result<()> {
        let prior = self.pause_state.lock().await.take();
        self.pause_active.store(false, Ordering::SeqCst);

        // Manual resume aborts the timer; the auto path is the timer
        // itself, so aborting would no-op anyway.
        if !auto {
            if let Some(existing) = self.pause_timer.lock().await.take() {
                existing.abort();
            }
        } else {
            // Drop the handle entry without abort so the slot is free
            // for the next pause cycle.
            self.pause_timer.lock().await.take();
        }

        let should_restart = prior.map(|p| p.was_running).unwrap_or(false);
        tracing::info!(auto, should_restart, "capture_pipeline.resumed");
        // Fire the listener BEFORE attempting restart so the tray menu
        // flips to "Pause for 1 hour / 1 day" immediately; if start()
        // fails, the menu shape still matches the (no-longer-paused)
        // state.
        self.notify_pause_change();
        if should_restart && !self.is_running() {
            if let Err(error) = self.start().await {
                tracing::error!(?error, "capture_pipeline.resume_start_failed");
                return Err(error);
            }
        }
        Ok(())
    }

    /// Quick Ask invoke (spec §六 + §八). Synchronous capture path —
    /// runs phase A (probe + screenshot) and phase B (extractor + ingest)
    /// back-to-back, returning the full focus context. Used by the
    /// `quick_ask_capture_focus` IPC fallback for direct window opens.
    ///
    /// The hotkey path in `lib.rs` calls `invoke_quick_ask_phase_a` and
    /// `invoke_quick_ask_phase_b` separately so it can show the window
    /// after phase A and run phase B in the background — the AX walk
    /// can blow past 1s and we don't want it gating perceived latency.
    pub async fn invoke_quick_ask(&self) -> Result<crate::domain::focus_context::FocusContext> {
        match self.invoke_quick_ask_phase_a().await? {
            QuickAskPhaseAOutcome::Excluded(focus) => Ok(focus),
            QuickAskPhaseAOutcome::Pending { state, .. } => {
                self.invoke_quick_ask_phase_b(state).await
            }
        }
    }

    /// Phase A — fast: probe foreground, exclusion check, screenshot,
    /// save to disk. Excluded apps short-circuit and a full
    /// [`FocusContext`] (with `excluded=true`) is returned immediately.
    /// For included apps, returns a [`QuickAskSkeleton`] (the bits the
    /// FocusCard can render right away) plus a [`QuickAskPendingState`]
    /// the caller hands back to [`Self::invoke_quick_ask_phase_b`].
    pub async fn invoke_quick_ask_phase_a(&self) -> Result<QuickAskPhaseAOutcome> {
        if self.is_paused() {
            return Err(CorivoError::Internal(
                "CapturePipeline is paused; Quick Ask blocked until resume".into(),
            ));
        }
        let phase_a_start = std::time::Instant::now();
        let probe = foreground::probe_current().await;
        tracing::debug!(
            app = ?probe.name,
            bundle = ?probe.bundle_id,
            pid = ?probe.pid,
            "quick_ask.phase_a.probe_done"
        );

        let exclusion_verdict = self
            .exclusion
            .lock()
            .await
            .check(probe.bundle_id.as_deref());
        // Both Block and BlockSelf surface to the user as "this app is
        // excluded". The lib.rs hotkey handler captures the prior
        // foreground before showing the Quick Ask window, so BlockSelf
        // shouldn't normally fire here — but if Corivo is already the
        // foreground, we still degrade gracefully instead of trying to
        // capture our own UI.
        if let Some(reason) = match &exclusion_verdict {
            crate::services::exclusion::ExclusionVerdict::Block { reason } => Some(reason.clone()),
            crate::services::exclusion::ExclusionVerdict::BlockSelf => Some(format!(
                "app:{}",
                probe.bundle_id.as_deref().unwrap_or("ai.corivo.desktop")
            )),
            crate::services::exclusion::ExclusionVerdict::Allow => None,
        } {
            let session_id = self.quick_ask_session_id().await;
            let env = build_skipped_envelope_with_trigger(
                &probe,
                &self.device_id,
                &session_id,
                reason,
                "exclusion".into(),
                Trigger::QuickAsk,
            );
            let captured_at = env.captured_at;
            let frame_id = self.ingest_and_get_id(env).await?;
            tracing::info!(
                total_ms = phase_a_start.elapsed().as_millis() as u64,
                "quick_ask.phase_a.excluded_done"
            );
            return Ok(QuickAskPhaseAOutcome::Excluded(
                crate::domain::focus_context::FocusContext {
                    frame_id,
                    captured_at,
                    app_bundle_id: probe.bundle_id,
                    app_name: probe.name,
                    window_title: probe.window_title,
                    url: probe.url,
                    adapter_name: None,
                    adapter_payload: None,
                    primary_text: String::new(),
                    selection: None,
                    excluded: true,
                    empty: false,
                },
            ));
        }

        let session_id = self.quick_ask_session_id().await;
        let skeleton = crate::domain::focus_context::QuickAskSkeleton {
            captured_at: chrono::Utc::now(),
            app_bundle_id: probe.bundle_id.clone(),
            app_name: probe.name.clone(),
            window_title: probe.window_title.clone(),
            url: probe.url.clone(),
        };
        let state = QuickAskPendingState { probe, session_id };
        tracing::info!(
            total_ms = phase_a_start.elapsed().as_millis() as u64,
            "quick_ask.phase_a.pending_done"
        );
        Ok(QuickAskPhaseAOutcome::Pending { skeleton, state })
    }

    /// Phase B — slow: per-app adapter pipeline (potentially an AX
    /// walk up to 1500 ms). If AX comes back empty the extractor takes
    /// a screenshot on demand via the helper sidecar + OCRs it; the
    /// AX-only happy path never touches the screen. Builds the
    /// snapshot envelope and ingests the frame, returning the full
    /// [`FocusContext`] with `frame_id` and `primary_text` populated.
    pub async fn invoke_quick_ask_phase_b(
        &self,
        state: QuickAskPendingState,
    ) -> Result<crate::domain::focus_context::FocusContext> {
        let phase_b_start = std::time::Instant::now();
        let QuickAskPendingState { probe, session_id } = state;

        let captured_at = chrono::Utc::now();
        let cfg = self.config.lock().await.clone();

        // Quick Ask routes through the per-app adapter pipeline (spec
        // §六: "Quick Ask invoke 触发的 frame 永远走 adapter 路径").
        let step = std::time::Instant::now();
        let outcome = extractor::extract(&extractor::ExtractInput {
            app_bundle_id: probe.bundle_id.clone(),
            app_name: probe.name.clone(),
            window_title: probe.window_title.clone(),
            window_pid: probe.pid,
            captured_at,
            trigger: Trigger::QuickAsk,
            adapters: Some(self.adapters.clone()),
            ocr_resources: Some(OcrResources {
                capture_store: self.capture_store.clone(),
                session_id: session_id.clone(),
                jpeg_quality: cfg.jpeg_quality,
            }),
        })
        .await
        .map_err(|e| CorivoError::Internal(format!("quick_ask extractor: {e}")))?;
        tracing::info!(
            elapsed_ms = step.elapsed().as_millis() as u64,
            "quick_ask.phase_b.extract_done"
        );

        let primary_text = outcome
            .ax_text
            .clone()
            .or_else(|| outcome.ocr_text.clone())
            .unwrap_or_default();
        let empty = primary_text.trim().is_empty();
        let env = SnapshotEnvelope {
            v: ENVELOPE_VERSION,
            captured_at,
            device_id: self.device_id.clone(),
            session_id: session_id.clone(),
            frame: FrameSnapshot {
                app: AppInfo {
                    bundle_id: probe.bundle_id.clone(),
                    name: probe.name.clone(),
                    pid: probe.pid,
                },
                window: WindowInfo {
                    title: probe.window_title.clone(),
                    is_focused: true,
                },
                url: probe.url.clone(),
                screenshot: outcome.screenshot.clone(),
            },
            extraction: ExtractionResult {
                strategy: outcome.strategy,
                ax_text: outcome.ax_text.clone(),
                ocr_text: outcome.ocr_text.clone(),
                adapter_name: outcome.adapter_name.clone(),
                adapter_payload: outcome.adapter_payload.clone(),
                selection: outcome.selection.clone(),
                duration_ms: outcome.duration_ms,
                fallback_reason: outcome.fallback_reason.clone(),
            },
            trigger: Trigger::QuickAsk,
            raw: RawDebug::default(),
        };
        let frame_id = self.ingest_and_get_id(env).await?;
        tracing::info!(
            total_ms = phase_b_start.elapsed().as_millis() as u64,
            "quick_ask.phase_b.done"
        );

        Ok(crate::domain::focus_context::FocusContext {
            frame_id,
            captured_at,
            app_bundle_id: probe.bundle_id,
            app_name: probe.name,
            window_title: probe.window_title,
            url: probe.url,
            adapter_name: outcome.adapter_name,
            adapter_payload: outcome.adapter_payload,
            primary_text,
            selection: outcome.selection,
            excluded: false,
            empty,
        })
    }

    /// Resolve which session id Quick Ask should write under. If the
    /// timer is running, share its session so Quick Ask frames sit
    /// alongside timer frames in `/timeline`. Otherwise lazy-create a
    /// dedicated session so the FK target is valid.
    async fn quick_ask_session_id(&self) -> String {
        if let Some(active) = self.session_id.lock().await.clone() {
            return active;
        }
        let mut slot = self.quick_ask_session.lock().await;
        if let Some(existing) = slot.as_ref() {
            return existing.clone();
        }
        let info = self
            .capture_store
            .create_session(0, 1)
            .await
            .expect("CaptureStore.create_session(quick_ask)");
        let id = info.id.clone();
        *slot = Some(id.clone());
        id
    }

    /// Ingest the envelope and recover the inserted frame id by
    /// re-querying the session. LocalConsumer doesn't expose the
    /// inserted row directly; Quick Ask is single-threaded per invoke
    /// so the race against a concurrent timer tick is benign in
    /// practice.
    async fn ingest_and_get_id(&self, env: SnapshotEnvelope) -> Result<String> {
        let session_id = env.session_id.clone();
        self.consumer.ingest(env).await?;
        let last = self
            .frames
            .last_in_session(&session_id)
            .await?
            .ok_or_else(|| {
                CorivoError::Internal(
                    "quick_ask: ingest succeeded but last_in_session returned None".into(),
                )
            })?;
        Ok(last.id)
    }
}

/// Public so [`coordinator::run_jobs`] can call it.
///
/// `trigger` decides what's stamped into the resulting frame row.
/// `adapters`, when `Some`, lets the dispatcher route through the
/// per-app adapter pipeline. Today the event driver always passes
/// `Some`; Quick Ask wires its own call site (also `Some`).
///
/// No pre-emptive screenshot here — the extractor takes one on demand
/// only when OCR fallback fires. AX-only ticks never touch the screen.
pub(crate) async fn tick(
    cfg: &CapturePipelineConfig,
    consumer: &dyn SnapshotConsumer,
    frames: &dyn FrameRepo,
    capture_store: Arc<CaptureStore>,
    exclusion: &ExclusionEngine,
    adapters: Option<&Arc<crate::services::extractor::AdapterRegistry>>,
    device_id: &str,
    session_id: &str,
    trigger: Trigger,
) -> Result<()> {
    // 1. Idle skip --------------------------------------------------------
    if let Some(idle) = screen_capture::current_idle_seconds() {
        if idle >= cfg.idle_skip_secs {
            tracing::debug!(idle, "capture_pipeline.idle_skip");
            return Ok(());
        }
    }

    // 2. Probe foreground -------------------------------------------------
    let probe = foreground::probe_current().await;

    // 3. Exclusion check --------------------------------------------------
    let verdict = exclusion.check(probe.bundle_id.as_deref());

    match verdict {
        // Corivo itself — drop without a frame row. Reading our own
        // window produces a recursive context and an "I was in Corivo
        // at this time" timeline entry isn't useful.
        ExclusionVerdict::BlockSelf => {
            tracing::debug!(
                bundle = ?probe.bundle_id,
                "capture_pipeline.skip_self"
            );
            return Ok(());
        }
        ExclusionVerdict::Block { reason } => {
            let env = build_skipped_envelope_with_trigger(
                &probe,
                device_id,
                session_id,
                reason,
                "exclusion".into(),
                trigger,
            );
            consumer.ingest(env).await?;
            return Ok(());
        }
        ExclusionVerdict::Allow => {}
    }

    // TODO(website-exclusion): once `foreground::probe_current()` populates
    // `probe.url`, insert a check against
    // `services::website_exclusion::WebsiteExclusionEngine` here and
    // emit a `'website:<host>'`-reasoned skipped frame on a hit. The
    // engine + Tauri CRUD already live in tree
    // (`services/website_exclusion`, `commands/capture.rs::website_exclusion_*`);
    // only the live filter plumbing is deferred to avoid dead code
    // while URL data isn't available.

    let captured_at = chrono::Utc::now();

    // 4. Dedup ladder (metadata-only) -------------------------------------
    let last = frames.last_in_session(session_id).await?;
    let metadata = CurrentMetadata {
        app_bundle_id: probe.bundle_id.as_deref(),
        window_title: probe.window_title.as_deref(),
        url: probe.url.as_deref(),
    };

    if let DedupDecision::BumpStillPresent { last_frame_id } =
        classify_metadata(last.as_ref(), &metadata)
    {
        frames
            .touch_still_present(&last_frame_id, captured_at)
            .await?;
        tracing::debug!(
            last_frame_id = %last_frame_id,
            "capture_pipeline.dedup_bump_still_present"
        );
        return Ok(());
    }

    // 5. Extract (AX first; extractor takes a screenshot on demand for
    //    the OCR fallback path only) --------------------------------------
    let outcome = extractor::extract(&ExtractInput {
        app_bundle_id: probe.bundle_id.clone(),
        app_name: probe.name.clone(),
        window_title: probe.window_title.clone(),
        window_pid: probe.pid,
        captured_at,
        trigger,
        adapters: adapters.cloned(),
        ocr_resources: Some(OcrResources {
            capture_store: capture_store.clone(),
            session_id: session_id.to_string(),
            jpeg_quality: cfg.jpeg_quality,
        }),
    })
    .await
    .map_err(|e| CorivoError::Internal(format!("extractor failed: {e}")))?;

    // 6. Envelope + ingest ------------------------------------------------
    let env = SnapshotEnvelope {
        v: ENVELOPE_VERSION,
        captured_at,
        device_id: device_id.to_string(),
        session_id: session_id.to_string(),
        frame: FrameSnapshot {
            app: AppInfo {
                bundle_id: probe.bundle_id.clone(),
                name: probe.name.clone(),
                pid: probe.pid,
            },
            window: WindowInfo {
                title: probe.window_title.clone(),
                is_focused: true,
            },
            url: probe.url.clone(),
            screenshot: outcome.screenshot.clone(),
        },
        extraction: ExtractionResult {
            strategy: outcome.strategy,
            ax_text: outcome.ax_text,
            ocr_text: outcome.ocr_text,
            adapter_name: outcome.adapter_name,
            adapter_payload: outcome.adapter_payload,
            selection: outcome.selection,
            duration_ms: outcome.duration_ms,
            fallback_reason: outcome.fallback_reason,
        },
        trigger,
        raw: RawDebug::default(),
    };

    consumer.ingest(env).await?;
    Ok(())
}

fn build_skipped_envelope_with_trigger(
    probe: &foreground::ForegroundProbe,
    device_id: &str,
    session_id: &str,
    exclusion_match: String,
    fallback_reason: String,
    trigger: Trigger,
) -> SnapshotEnvelope {
    SnapshotEnvelope {
        v: ENVELOPE_VERSION,
        captured_at: chrono::Utc::now(),
        device_id: device_id.to_string(),
        session_id: session_id.to_string(),
        frame: FrameSnapshot {
            app: AppInfo {
                bundle_id: probe.bundle_id.clone(),
                name: probe.name.clone(),
                pid: probe.pid,
            },
            window: WindowInfo {
                title: probe.window_title.clone(),
                is_focused: true,
            },
            url: probe.url.clone(),
            screenshot: None,
        },
        extraction: ExtractionResult {
            strategy: ExtractionStrategy::Skipped,
            ax_text: None,
            ocr_text: None,
            adapter_name: None,
            adapter_payload: None,
            selection: None,
            duration_ms: 0,
            fallback_reason: Some(fallback_reason),
        },
        trigger,
        raw: RawDebug {
            ax_tree_json_path: None,
            exclusion_match: Some(exclusion_match),
        },
    }
}
