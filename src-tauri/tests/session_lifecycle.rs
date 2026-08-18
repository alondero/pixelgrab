//! Integration tests for the session lifecycle. Drives the orchestrator
//! through the full capture => select => commit => idle loop.

use std::sync::Arc;

use pixelgrab_contracts::capture::{CaptureFormat, CaptureRequest};
use pixelgrab_contracts::coordinate::PhysicalBounds;
use pixelgrab_contracts::session::{SessionState, SessionTransition, SessionTransitionReason};
use pixelgrab_lib::platform::synthetic::SyntheticPlatform;
use pixelgrab_lib::platform::PixelGrabPlatform;
use pixelgrab_lib::SessionOrchestrator;

#[test]
fn capture_session_walks_full_lifecycle() {
    let platform: Arc<dyn PixelGrabPlatform> = Arc::new(SyntheticPlatform::new());
    let session = SessionOrchestrator::new(platform);

    assert_eq!(session.current_state(), SessionState::Idle);

    let request = CaptureRequest {
        format: CaptureFormat::VirtualDesktop,
        monitor_id: None,
        region: None,
    };
    let capture = session.run_capture(&request).expect("capture succeeds");
    assert_eq!(capture.format, CaptureFormat::VirtualDesktop);
    assert_eq!(session.current_state(), SessionState::Ready);

    session.begin_selecting().expect("begin_selecting");
    assert_eq!(session.current_state(), SessionState::Selecting);

    let bounds = PhysicalBounds::from_xywh(100, 100, 800, 600);
    session.report_selection(bounds).expect("report_selection");
    assert_eq!(session.selection(), Some(bounds));

    session.finish().expect("finish");
    assert_eq!(session.current_state(), SessionState::Idle);
}

#[test]
fn illegal_transition_is_rejected() {
    let platform: Arc<dyn PixelGrabPlatform> = Arc::new(SyntheticPlatform::new());
    let session = SessionOrchestrator::new(platform);

    // Jumping straight from Idle to Selecting must fail.
    let result = session.request_transition(SessionTransition {
        to: SessionState::Selecting,
        reason: SessionTransitionReason::OverlayShown,
    });
    assert!(result.is_err(), "illegal transition should be rejected");
}

#[test]
fn reset_returns_to_idle() {
    let platform: Arc<dyn PixelGrabPlatform> = Arc::new(SyntheticPlatform::new());
    let session = SessionOrchestrator::new(platform);

    session
        .request_transition(SessionTransition {
            to: SessionState::Capturing,
            reason: SessionTransitionReason::CaptureRequested,
        })
        .unwrap();
    session.reset();
    assert_eq!(session.current_state(), SessionState::Idle);
}

#[test]
fn snapshot_reflects_capture_resolution() {
    let platform: Arc<dyn PixelGrabPlatform> = Arc::new(SyntheticPlatform::new());
    let session = SessionOrchestrator::new(platform);

    let request = CaptureRequest {
        format: CaptureFormat::VirtualDesktop,
        monitor_id: None,
        region: None,
    };
    session.run_capture(&request).unwrap();
    let snapshot = session.snapshot();
    assert_eq!(snapshot.state, SessionState::Ready);
    assert!(snapshot.last_capture.is_some());
    assert!(snapshot.selection.is_none());
}
