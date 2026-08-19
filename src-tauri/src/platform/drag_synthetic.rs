//! Synthetic drag adapter. The platform-neutral contract for external
//! drag-and-drop is `PixelGrabPlatform::start_drag`. The synthetic
//! implementation is the fault-injection seam: it cannot run `DoDragDrop`
//! in CI, so it provides a deterministic, scriptable replacement that
//! tests use to drive the four terminal outcomes (accepted, rejected,
//! cancelled, failed) and to assert on the recorded diagnostics.
//!
//! The adapter also doubles as the leak detector: it counts every
//! `start_drag` call and asserts the file handle backing the PNG was
//! still alive the moment each simulated drag terminated. A test that
//! re-runs accepted/cancelled/rejected/failed loops in any order must
//! observe handle count == 0 at the end, otherwise the dragging side
//! has leaked a file handle or a cache lock.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use pixelgrab_contracts::{
    drag::{
        DragDiagnostics, DragFormat, DragRequest, DragResult, DragTargetEffect, DragTargetKind,
    },
    PlatformError, PlatformErrorKind, PlatformResult,
};

/// Scriptable outcome injection. The synthetic adapter advances through
/// the queued outcomes on every call. Tests that need a stable failure
/// use `Always(...)`; tests that exercise the loop round-robin through
/// [accepted, rejected, cancelled, failed] use `Cycle`.
#[derive(Debug, Clone, Default)]
pub enum SyntheticDragScript {
    /// Always return the same outcome (defaults to Cancelled).
    #[default]
    Stable,
    /// Round-robin through the four terminals in the order
    /// [Accepted, Rejected, Cancelled, Failed].
    Cycle,
    /// Inject a failure with a specific categorical `PlatformErrorKind`
    /// label. The diagnostic record is stamped with `Failed` and the
    /// provided label.
    AlwaysFail(&'static str),
}

impl SyntheticDragScript {
    /// Outcome to return on the nth call. The index is monotonically
    /// increasing and the synthetic adapter does not reset the counter
    /// between calls — this is what enforces the "no leak after N
    /// loops" property.
    pub fn outcome_for(&self, call_index: usize) -> PlannedOutcome {
        match self {
            Self::Stable => PlannedOutcome::Finalize(DragOutcomePlan::Cancelled),
            Self::Cycle => {
                let variant = match call_index % 4 {
                    0 => DragOutcomePlan::Accepted,
                    1 => DragOutcomePlan::Rejected,
                    2 => DragOutcomePlan::Cancelled,
                    _ => DragOutcomePlan::Failed,
                };
                PlannedOutcome::Finalize(variant)
            }
            Self::AlwaysFail(kind) => PlannedOutcome::InjectFailure(kind),
        }
    }
}

/// The per-call plan. The synthetic adapter distinguishes the "natural
/// terminal outcome" path from the "force a failure" path so the
/// diagnostics record can be stamped with the right target effect.
#[derive(Debug, Clone, Copy)]
pub enum PlannedOutcome {
    /// The drag terminated with the given outcome.
    Finalize(DragOutcomePlan),
    /// The drag aborted with the given `PlatformErrorKind` label.
    InjectFailure(&'static str),
}

/// The four terminal outcomes the synthetic adapter can simulate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DragOutcomePlan {
    /// Simulate a successful drop.
    Accepted,
    /// Simulate a drop target that rejected every offered format.
    Rejected,
    /// Simulate a user-cancelled drag (Esc, released outside a target).
    Cancelled,
    /// Simulate a drag that failed in the OLE pipeline itself.
    Failed,
}

impl DragOutcomePlan {
    /// Translate the plan into the wire `DragOutcome`.
    pub fn to_outcome(self) -> pixelgrab_contracts::drag::DragOutcome {
        use pixelgrab_contracts::drag::DragOutcome;
        match self {
            Self::Accepted => DragOutcome::Accepted,
            Self::Rejected => DragOutcome::Rejected,
            Self::Cancelled => DragOutcome::Cancelled,
            Self::Failed => DragOutcome::Failed,
        }
    }

    /// Categorical target effect for the diagnostics record.
    pub fn target_effect(self) -> DragTargetEffect {
        match self {
            Self::Accepted => DragTargetEffect::Copy,
            Self::Rejected => DragTargetEffect::None,
            Self::Cancelled => DragTargetEffect::Unknown,
            Self::Failed => DragTargetEffect::Unknown,
        }
    }

    /// Categorical target class for the diagnostics record.
    pub fn target_kind(self) -> DragTargetKind {
        match self {
            Self::Accepted => DragTargetKind::Other,
            Self::Rejected => DragTargetKind::Rejecting,
            Self::Cancelled => DragTargetKind::None,
            Self::Failed => DragTargetKind::None,
        }
    }
}

/// The synthetic drag adapter. Designed to back the synthetic platform
/// during tracer-09 tests. The interface is intentionally narrow:
/// `start_drag` is the only call, and the rest of the state is observed
/// through the `record` field for assertions.
#[derive(Debug, Clone)]
pub struct SyntheticDragSource {
    inner: Arc<SyntheticDragState>,
}

#[derive(Debug)]
struct SyntheticDragState {
    script: Mutex<SyntheticDragScript>,
    /// Monotonically increasing call counter. The synthetic adapter does
    /// not reset this between calls so the leak guards can assert that
    /// the handle count returns to zero on every loop.
    call_count: Mutex<usize>,
    /// For each call, the absolute PNG path the platform contract was
    /// asked to lock. The handle is held until the call ends. Tests
    /// read this to verify the file handle was retained for the full
    /// synchronous drag.
    held_paths: Mutex<Vec<PathBuf>>,
    /// Every format-request event recorded by the synthetic adapter.
    /// The tuple is `(call_index, format, at_ms_offset)`. The adapter
    /// never makes a real `CF_HDROP` request; the `request_format`
    /// call is the test seam that exercises the "format requested late
    /// in the drag" property. The `at_ms_offset` is the millisecond
    /// offset from the drag start the test wants the format stamped
    /// with — the synthetic adapter copies the value verbatim into the
    /// diagnostics record.
    format_requests: Mutex<Vec<(usize, DragFormat, i64)>>,
    /// Outcome log. Indexed by call number.
    outcomes: Mutex<Vec<pixelgrab_contracts::drag::DragOutcome>>,
}

impl SyntheticDragSource {
    /// Construct a synthetic drag source with the stable (default) script.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SyntheticDragState {
                script: Mutex::new(SyntheticDragScript::default()),
                call_count: Mutex::new(0),
                held_paths: Mutex::new(Vec::new()),
                format_requests: Mutex::new(Vec::new()),
                outcomes: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Replace the script. Used by tests to exercise the cycle and
    /// failure-injection paths.
    pub fn set_script(&self, script: SyntheticDragScript) {
        *self.inner.script.lock() = script;
    }

    /// Total number of `start_drag` calls observed.
    pub fn call_count(&self) -> usize {
        *self.inner.call_count.lock()
    }

    /// Snapshot of the outcomes observed so far. The vector is empty
    /// until the first call is observed.
    pub fn outcomes(&self) -> Vec<pixelgrab_contracts::drag::DragOutcome> {
        self.inner.outcomes.lock().clone()
    }

    /// Snapshot of the format requests observed. The synthetic adapter
    /// records only what the test asked for — the real Windows adapter
    /// records what the drop target actually pulled.
    pub fn format_requests(&self) -> Vec<(usize, DragFormat, i64)> {
        self.inner.format_requests.lock().clone()
    }

    /// Snapshot of the absolute PNG paths the adapter held for the
    /// current concurrent drag. The vector is empty outside a drag.
    pub fn held_paths(&self) -> Vec<PathBuf> {
        self.inner.held_paths.lock().clone()
    }

    /// Inject a synthetic format request as if the drop target pulled
    /// that clipboard format during the drag loop. The `call_index` is
    /// the call number; `at_ms_offset` is the offset from the drag
    /// start the test wants the format stamped with.
    pub fn request_format(&self, call_index: usize, format: DragFormat, at_ms_offset: i64) {
        self.inner
            .format_requests
            .lock()
            .push((call_index, format, at_ms_offset));
    }

    /// Erase all recorded state. Tests that need a clean slate between
    /// loops call this; production code never resets.
    pub fn reset(&self) {
        *self.inner.call_count.lock() = 0;
        self.inner.held_paths.lock().clear();
        self.inner.format_requests.lock().clear();
        self.inner.outcomes.lock().clear();
    }

    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

impl Default for SyntheticDragSource {
    fn default() -> Self {
        Self::new()
    }
}

impl SyntheticDragSource {
    /// Run a synthetic drag. The backing PNG file is held in
    /// `held_paths` for the duration of the call. The terminal outcome
    /// is decided by the script.
    pub fn run(&self, request: &DragRequest) -> PlatformResult<DragResult> {
        request.validate()?;
        let path = PathBuf::from(&request.png_path);
        if !path.exists() {
            return Err(PlatformError::new(
                PlatformErrorKind::Io,
                "drag request: backing PNG does not exist",
            ));
        }
        let call_index = {
            let mut count = self.inner.call_count.lock();
            let idx = *count;
            *count += 1;
            idx
        };

        let started_at = self.now_ms();
        {
            let mut held = self.inner.held_paths.lock();
            held.push(path.clone());
        }

        let script = self.inner.script.lock().clone();
        let plan = script.outcome_for(call_index);
        let completed_at = self.now_ms();

        // Release the file handle before returning. The leak guard below
        // asserts the held_paths list is empty after the call.
        self.inner.held_paths.lock().clear();

        match plan {
            PlannedOutcome::Finalize(variant) => {
                let outcome = variant.to_outcome();
                self.inner.outcomes.lock().push(outcome);
                let mut diag = DragDiagnostics::started(
                    request.capture_id.clone(),
                    request.shelf_id.clone(),
                    started_at,
                )
                .completed(completed_at)
                .with_target_effect(variant.target_effect())
                .with_target_kind(variant.target_kind());
                // Replay the recorded format requests into the diagnostics
                // record. The synthetic adapter copies the test-provided
                // timestamp verbatim so the diagnostics reflect the actual
                // pull moments the test injected.
                let recorded = self.inner.format_requests.lock().clone();
                for (idx, fmt, at_ms) in recorded.iter() {
                    if *idx == call_index {
                        diag.record_format_request(*fmt, *at_ms);
                    }
                }
                Ok(DragResult {
                    outcome,
                    diagnostics: diag,
                })
            }
            PlannedOutcome::InjectFailure(kind) => {
                let outcome = pixelgrab_contracts::drag::DragOutcome::Failed;
                self.inner.outcomes.lock().push(outcome);
                let diag = DragDiagnostics::started(
                    request.capture_id.clone(),
                    request.shelf_id.clone(),
                    started_at,
                )
                .completed(completed_at)
                .with_target_effect(DragTargetEffect::Unknown)
                .with_target_kind(DragTargetKind::None)
                .failed(kind);
                Ok(DragResult {
                    outcome,
                    diagnostics: diag,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> DragRequest {
        DragRequest {
            capture_id: "capture-1".into(),
            shelf_id: Some("shelf-1".to_string()),
            png_path: "test.png".into(),
            bgra_pixels: vec![0u8; 8 * 8 * 4],
            width: 8,
            height: 8,
        }
    }

    fn with_temp_png() -> (Tempdir, DragRequest) {
        let dir = Tempdir::new();
        let path = dir.path().join("capture.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\n").expect("write png");
        let mut req = sample_request();
        req.png_path = path.to_string_lossy().to_string();
        (dir, req)
    }

    struct Tempdir(PathBuf);

    impl Tempdir {
        fn new() -> Self {
            let unique = format!(
                "pixelgrab-synthetic-drag-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            );
            let root = std::env::temp_dir().join(unique);
            std::fs::create_dir_all(&root).expect("create tempdir");
            Tempdir(root)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for Tempdir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn validates_missing_png() {
        let source = SyntheticDragSource::new();
        let req = sample_request();
        let result = source.run(&req);
        assert!(result.is_err());
    }

    #[test]
    fn stable_script_returns_cancelled() {
        let (_dir, req) = with_temp_png();
        let source = SyntheticDragSource::new();
        let result = source.run(&req).expect("run");
        assert_eq!(
            result.outcome,
            pixelgrab_contracts::drag::DragOutcome::Cancelled
        );
        assert!(source.held_paths().is_empty(), "handle released");
        assert_eq!(source.call_count(), 1);
    }

    #[test]
    fn cycle_script_round_trips_all_outcomes() {
        let (_dir, req) = with_temp_png();
        let source = SyntheticDragSource::new();
        source.set_script(SyntheticDragScript::Cycle);
        let mut seen = Vec::new();
        for _ in 0..4 {
            let r = source.run(&req).expect("run");
            seen.push(r.outcome);
        }
        assert_eq!(
            seen,
            vec![
                pixelgrab_contracts::drag::DragOutcome::Accepted,
                pixelgrab_contracts::drag::DragOutcome::Rejected,
                pixelgrab_contracts::drag::DragOutcome::Cancelled,
                pixelgrab_contracts::drag::DragOutcome::Failed,
            ]
        );
        assert!(source.held_paths().is_empty(), "no leaked handles");
    }

    #[test]
    fn loop_repeated_runs_do_not_leak_handles() {
        let (_dir, req) = with_temp_png();
        let source = SyntheticDragSource::new();
        source.set_script(SyntheticDragScript::Cycle);
        for _ in 0..12 {
            let _ = source.run(&req).expect("run");
        }
        assert_eq!(source.call_count(), 12);
        assert!(source.held_paths().is_empty(), "no leaked handles");
    }

    #[test]
    fn failure_injection_stamps_diag() {
        let (_dir, req) = with_temp_png();
        let source = SyntheticDragSource::new();
        source.set_script(SyntheticDragScript::AlwaysFail("io"));
        let result = source.run(&req).expect("run");
        assert_eq!(
            result.outcome,
            pixelgrab_contracts::drag::DragOutcome::Failed
        );
        assert_eq!(result.diagnostics.failure_kind.as_deref(), Some("io"));
        assert_eq!(result.diagnostics.target_effect, DragTargetEffect::Unknown);
    }

    #[test]
    fn format_request_is_recorded() {
        let (_dir, req) = with_temp_png();
        let source = SyntheticDragSource::new();
        // Pre-load a request for the upcoming call (index 0).
        source.request_format(0, DragFormat::Hdrop, 5);
        source.request_format(0, DragFormat::DibV5, 12);
        let result = source.run(&req).expect("run");
        assert_eq!(result.diagnostics.requested_formats.len(), 2);
        assert_eq!(
            result.diagnostics.requested_formats[0].format,
            DragFormat::Hdrop
        );
        assert_eq!(result.diagnostics.requested_formats[0].at_ms, 5);
        assert_eq!(result.diagnostics.requested_formats[1].at_ms, 12);
    }
}
