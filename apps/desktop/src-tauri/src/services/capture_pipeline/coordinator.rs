//! Event-driver coordinator.
//!
//! Single async task that owns the [`Debouncer`] and translates
//! [`Event`]s into [`SnapshotJob`]s. The job runner (a separate task,
//! see [`run_jobs`]) consumes the jobs and calls into the same
//! `tick()` body the timer driver uses.
//!
//! Why split into coordinator + runner instead of one task that does
//! both: the coordinator must stay responsive to incoming events
//! (especially SafetyNet ticks and rapid AppActivated bursts). If we
//! ran `tick()` inline, a slow AX traversal or OCR pass would block
//! the bus and we'd lose the debouncer's reset semantics on whichever
//! events arrived during the stall.
//!
//! Exclusion checks happen *inside* `tick()`, not the coordinator —
//! we keep one implementation of "this app is excluded → write a
//! skipped frame" and reuse it.

use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use tokio::sync::{mpsc, Mutex};

use crate::{
    db::repos::frames::FrameRepo,
    domain::snapshot_envelope::Trigger,
    services::{
        capture_pipeline::{
            debouncer::{default_debounce, Debouncer, SnapshotJob},
            event_bus::Event,
            tick, CapturePipelineConfig,
        },
        capture_store::CaptureStore,
        exclusion::ExclusionEngine,
        extractor::AdapterRegistry,
        snapshot_consumer::SnapshotConsumer,
    },
};

/// Surface key used for events that aren't tied to a foreground app
/// (manual triggers). Distinct from any real bundle id so it lives
/// in its own debouncer slot.
const MANUAL_SURFACE_KEY: &str = "<manual>";

/// Channel buffer for jobs queued from coordinator → runner. The
/// runner is single-task (no parallel snapshots) so this lets a small
/// burst of events queue up while a slow AX traversal completes.
const JOB_QUEUE_CAPACITY: usize = 16;

/// Spawn the coordinator task. Returns the [`mpsc::Sender`] for the
/// runner so the caller can wire it up. Coordinator exits when the
/// event-bus channel closes or it receives `Event::Shutdown`.
pub fn spawn(
    bus: mpsc::Receiver<Event>,
) -> (mpsc::Receiver<SnapshotJob>, tokio::task::JoinHandle<()>) {
    let (job_tx, job_rx) = mpsc::channel::<SnapshotJob>(JOB_QUEUE_CAPACITY);
    let handle = tokio::spawn(coordinator_loop(bus, job_tx));
    (job_rx, handle)
}

/// Surface key for AX-derived events. The active app is implicit
/// (AXObserver only watches the front app), so all AX events share
/// one debouncer slot — this is what we want, because rapid
/// title-changes and focused-window-changes about the same window
/// should collapse into a single capture.
const AX_SURFACE_KEY: &str = "<ax_active>";

async fn coordinator_loop(mut bus: mpsc::Receiver<Event>, job_tx: mpsc::Sender<SnapshotJob>) {
    let mut debouncer = Debouncer::new(job_tx);
    while let Some(event) = bus.recv().await {
        match event {
            Event::AppActivated { from, to } => {
                // Skip the "no foreground app" transition — we have
                // nothing to capture and the next real activation will
                // wake us up anyway.
                let Some(bundle_id) = to else {
                    tracing::trace!(?from, "coordinator.app_activated.no_target");
                    continue;
                };
                let job = SnapshotJob {
                    trigger: Trigger::FocusChange,
                    surface_key: bundle_id.clone(),
                };
                debouncer.fire(bundle_id, job, default_debounce(Trigger::FocusChange));
            }
            Event::FocusedWindow => {
                let job = SnapshotJob {
                    trigger: Trigger::FocusedWindowChanged,
                    surface_key: AX_SURFACE_KEY.into(),
                };
                debouncer.fire(
                    AX_SURFACE_KEY.into(),
                    job,
                    default_debounce(Trigger::FocusedWindowChanged),
                );
            }
            Event::TitleChanged => {
                let job = SnapshotJob {
                    trigger: Trigger::TitleChanged,
                    surface_key: AX_SURFACE_KEY.into(),
                };
                debouncer.fire(
                    AX_SURFACE_KEY.into(),
                    job,
                    default_debounce(Trigger::TitleChanged),
                );
            }
            Event::ManualSnapshot { reason } => {
                tracing::debug!(%reason, "coordinator.manual_snapshot");
                let job = SnapshotJob {
                    trigger: Trigger::Manual,
                    surface_key: MANUAL_SURFACE_KEY.into(),
                };
                debouncer.fire(MANUAL_SURFACE_KEY.into(), job, Duration::ZERO);
            }
            Event::Shutdown => {
                tracing::info!("coordinator.shutdown_requested");
                break;
            }
        }
    }
    // Either the bus closed or we hit Shutdown — cancel any pending
    // debounce timers so we don't fire spurious jobs after stop.
    debouncer.drain();
    tracing::info!("coordinator.exit");
}

/// Job runner inputs. Mirrors the timer driver's `RunLoopArgs` so the
/// two callsites share their dependencies.
pub struct JobRunnerArgs {
    pub job_rx: mpsc::Receiver<SnapshotJob>,
    pub consumer: Arc<dyn SnapshotConsumer>,
    pub frames: Arc<dyn FrameRepo>,
    pub capture_store: Arc<CaptureStore>,
    pub exclusion: Arc<Mutex<ExclusionEngine>>,
    pub adapters: Arc<AdapterRegistry>,
    pub config: Arc<Mutex<CapturePipelineConfig>>,
    pub running: Arc<std::sync::atomic::AtomicBool>,
    pub device_id: String,
    pub session_id: String,
}

/// Job runner — single worker. Drains jobs, runs `tick()` per job
/// with the trigger the coordinator stamped on it. Exits when the
/// channel closes (coordinator gone) or `running` flips false.
pub async fn run_jobs(mut args: JobRunnerArgs) {
    while args.running.load(Ordering::SeqCst) {
        let Some(job) = args.job_rx.recv().await else {
            tracing::info!(session_id = %args.session_id, "job_runner.bus_closed");
            break;
        };

        let cfg_now = args.config.lock().await.clone();
        let exclusion_now = args.exclusion.lock().await.clone();

        if let Err(error) = tick(
            &cfg_now,
            &*args.consumer,
            &*args.frames,
            args.capture_store.clone(),
            &exclusion_now,
            Some(&args.adapters),
            &args.device_id,
            &args.session_id,
            job.trigger,
        )
        .await
        {
            tracing::error!(
                ?error,
                trigger = job.trigger.as_str(),
                surface_key = %job.surface_key,
                "job_runner.tick_failed"
            );
        }
    }
    tracing::info!(session_id = %args.session_id, "job_runner.exit");
}
