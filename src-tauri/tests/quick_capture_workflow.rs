#[cfg(test)]
mod workflow_tests {
    use pixelgrab_contracts::ipc::*;
    use pixelgrab_contracts::*;
    use pixelgrab_contracts::{
        Annotation, AnnotationColor, AnnotationStroke, CaptureIntent, PhysicalPoint, SessionState,
    };
    use pixelgrab_lib::{
        ipc::commands::{
            cancel_revision_native, capture_native, commit_capture, commit_revision_native,
            open_revision_context, open_revision_native, start_shelf_drag_native,
        },
        PixelGrabApp,
    };
    use pixelgrab_test_support::capture::FramePattern;
    use std::sync::Arc;
    use tauri::Manager;

    fn fixture() -> (
        PixelGrabApp,
        tauri::App<tauri::test::MockRuntime>,
        tempfile::TempDir,
    ) {
        let monitor = MonitorDescriptor {
            id: "test".into(),
            label: "Test display".into(),
            bounds: PhysicalBounds::from_xywh(-40, 0, 80, 60),
            work_area: PhysicalBounds::from_xywh(-40, 0, 80, 60),
            scale_factor: 1.0,
            is_primary: true,
        };
        let platform = Arc::new(
            pixelgrab_lib::platform::synthetic::SyntheticPlatform::with_layout(
                MonitorLayout::new(vec![monitor]),
                FramePattern::GradientWithWatermark,
            ),
        );
        let app = PixelGrabApp::new(platform, pixelgrab_lib::hotkey::InMemoryBackend::new());
        let root = tempfile::tempdir().unwrap();
        app.cache()
            .set_cache_root(Some(root.path().to_path_buf()))
            .unwrap();
        let runtime = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        (app, runtime, root)
    }

    #[test]
    fn full_screen_delivers_without_overlay_and_next_capture_succeeds() {
        let (app, runtime, _root) = fixture();
        for _ in 0..2 {
            let result = capture_native(
                &app,
                RequestCaptureIntent {
                    intent: CaptureIntent::FullScreen,
                },
                runtime.handle(),
            )
            .unwrap();
            assert_eq!(
                result.capture.bounds,
                PhysicalBounds::from_xywh(-40, 0, 80, 60)
            );
            assert_eq!(app.session().current_state(), SessionState::Idle);
        }
        let snapshot = app.shelf_queue().snapshot(0);
        assert_eq!(snapshot.cards.len(), 2);
        for card in snapshot.cards {
            assert!(std::path::Path::new(&card.png_path).is_file());
        }
        assert!(runtime.get_webview_window("overlay").is_none());
    }

    #[test]
    fn failed_immediate_delivery_leaves_no_card_and_can_retry() {
        let (app, runtime, _root) = fixture();
        app.cache().arm_failure(
            pixelgrab_lib::cache::CommitStage::WriteManifest,
            PlatformError::new(PlatformErrorKind::Io, "injected"),
        );
        assert!(capture_native(
            &app,
            RequestCaptureIntent {
                intent: CaptureIntent::FullScreen
            },
            runtime.handle()
        )
        .is_err());
        assert_eq!(app.session().current_state(), SessionState::Idle);
        assert!(app.shelf_queue().is_empty());
        assert!(app.cache().entries().is_empty());
        capture_native(
            &app,
            RequestCaptureIntent {
                intent: CaptureIntent::FullScreen,
            },
            runtime.handle(),
        )
        .unwrap();
        assert_eq!(app.shelf_queue().len(), 1);
    }

    #[test]
    fn overlay_reveal_failure_releases_frozen_session_and_allows_next_capture() {
        let (app, runtime, _root) = fixture();
        assert!(capture_native(
            &app,
            RequestCaptureIntent {
                intent: CaptureIntent::Region
            },
            runtime.handle()
        )
        .is_err());
        assert_eq!(app.session().current_state(), SessionState::Idle);
        assert!(app
            .session()
            .last_diagnostics()
            .unwrap()
            .failure_kind
            .is_some());
        assert!(app.shelf_queue().is_empty());
        capture_native(
            &app,
            RequestCaptureIntent {
                intent: CaptureIntent::FullScreen,
            },
            runtime.handle(),
        )
        .unwrap();
        assert_eq!(app.shelf_queue().len(), 1);
    }

    #[test]
    fn sharing_keeps_source_file_and_releases_only_the_temporary_drag_lock() {
        let (app, runtime, _root) = fixture();
        capture_native(
            &app,
            RequestCaptureIntent {
                intent: CaptureIntent::FullScreen,
            },
            runtime.handle(),
        )
        .unwrap();
        let card = app.shelf_queue().snapshot(0).cards[0].clone();
        start_shelf_drag_native(
            &app,
            &StartShelfDragIntent {
                shelf_id: card.shelf_id.clone(),
                dismiss_on_accepted: false,
            },
        )
        .unwrap();
        assert_eq!(app.shelf_queue().len(), 1);
        assert_eq!(
            app.cache().locks().owners_of(&card.shelf_id),
            vec![LockOwner::Shelf]
        );
        capture_native(
            &app,
            RequestCaptureIntent {
                intent: CaptureIntent::FullScreen,
            },
            runtime.handle(),
        )
        .unwrap();
        assert!(std::path::Path::new(&card.png_path).is_file());
        assert_eq!(app.shelf_queue().len(), 2);
    }

    #[test]
    fn tray_hotkey_and_secondary_capture_dispatch_does_not_need_a_main_webview() {
        let (app, runtime, _root) = fixture();
        runtime.manage(app);
        tauri::WebviewWindowBuilder::new(
            &runtime,
            "overlay",
            tauri::WebviewUrl::App("overlay.html".into()),
        )
        .build()
        .unwrap();
        pixelgrab_lib::singleton::forward_to_existing_instance(
            runtime.handle(),
            SecondaryLaunchIntent::CaptureRegion,
        );
        let state = runtime.state::<PixelGrabApp>();
        assert_eq!(state.session().current_state(), SessionState::Selecting);
        assert_eq!(
            state.session().last_capture().unwrap().format,
            CaptureFormat::VirtualDesktop
        );
        state.session().cancel_session().unwrap();
        pixelgrab_lib::singleton::forward_to_existing_instance(
            runtime.handle(),
            SecondaryLaunchIntent::CaptureFullScreen,
        );
        assert_eq!(state.session().current_state(), SessionState::Idle);
        assert_eq!(state.shelf_queue().len(), 1);
        assert!(runtime.get_webview_window("main").is_none());
    }

    #[test]
    fn editor_reveal_failure_releases_session_and_lock_then_native_reopen_can_close() {
        let (app, runtime, _root) = fixture();
        capture_native(
            &app,
            RequestCaptureIntent {
                intent: CaptureIntent::FullScreen,
            },
            runtime.handle(),
        )
        .unwrap();
        let shelf_id = app.shelf_queue().snapshot(0).cards[0].shelf_id.clone();
        let intent = OpenRevisionIntent {
            shelf_id: shelf_id.clone(),
        };
        assert!(open_revision_native(&app, intent.clone(), runtime.handle()).is_err());
        assert_eq!(app.session().current_state(), SessionState::Idle);
        assert!(!app.cache().has_editor_lock(&shelf_id));
        tauri::WebviewWindowBuilder::new(
            &runtime,
            "main",
            tauri::WebviewUrl::App("index.html".into()),
        )
        .build()
        .unwrap();
        open_revision_native(&app, intent, runtime.handle()).unwrap();
        assert_eq!(app.session().current_state(), SessionState::Reopening);
        assert!(app.cache().has_editor_lock(&shelf_id));
        cancel_revision_native(&app, &shelf_id).unwrap();
        assert_eq!(app.session().current_state(), SessionState::Idle);
        assert!(!app.cache().has_editor_lock(&shelf_id));
        capture_native(
            &app,
            RequestCaptureIntent {
                intent: CaptureIntent::FullScreen,
            },
            runtime.handle(),
        )
        .unwrap();
    }

    #[test]
    fn failed_revision_delivery_can_retry_without_changing_original_assets() {
        let (app, runtime, _root) = fixture();
        capture_native(
            &app,
            RequestCaptureIntent {
                intent: CaptureIntent::FullScreen,
            },
            runtime.handle(),
        )
        .unwrap();
        let shelf_id = app.shelf_queue().snapshot(0).cards[0].shelf_id.clone();
        let context = open_revision_context(
            &app,
            OpenRevisionIntent {
                shelf_id: shelf_id.clone(),
            },
        )
        .unwrap();
        let original = std::fs::read(&context.png_path).unwrap();
        let payload = CommitRevisionIntent {
            shelf_id: shelf_id.clone(),
            annotations: vec![],
            badge_counter: 1,
            active_tool: AnnotationTool::Select,
            active_color: AnnotationColor::Red,
            active_stroke: AnnotationStroke::Medium,
            metadata: CacheEntryMetadata::default(),
            to_clipboard: false,
        };
        app.cache().arm_failure(
            pixelgrab_lib::cache::CommitStage::WriteManifest,
            PlatformError::new(PlatformErrorKind::Io, "injected"),
        );
        assert!(commit_revision_native(&app, runtime.handle(), &payload).is_err());
        assert_eq!(app.session().current_state(), SessionState::Reopening);
        assert!(app.cache().has_editor_lock(&shelf_id));
        assert_eq!(std::fs::read(&context.png_path).unwrap(), original);
        assert!(capture_native(
            &app,
            RequestCaptureIntent {
                intent: CaptureIntent::FullScreen
            },
            runtime.handle()
        )
        .is_err());
        commit_revision_native(&app, runtime.handle(), &payload).unwrap();
        assert_eq!(app.session().current_state(), SessionState::Idle);
        assert_eq!(app.shelf_queue().len(), 2);
    }

    #[test]
    fn edited_capture_can_remove_annotations_and_reopen_without_double_flattening() {
        let (app, runtime, _root) = fixture();
        let capture = app
            .session()
            .request_capture(&CaptureRequest {
                format: CaptureFormat::SingleMonitor,
                monitor_id: Some("test".into()),
                region: None,
            })
            .unwrap();
        let annotation = Annotation::rectangle(
            pixelgrab_contracts::AnnotationId(1),
            PhysicalPoint::new(5, 5),
            PhysicalSize::new(20, 20),
            AnnotationColor::Red,
            AnnotationStroke::Medium,
            0,
        );
        let first = commit_capture(
            &app,
            runtime.handle(),
            &CommitRequest {
                crop: capture.bounds,
                annotations: vec![annotation],
                to_shelf: true,
                to_clipboard: false,
                save_as: false,
            },
        )
        .unwrap();
        let shelf_id = first.shelf_id.unwrap();
        let context = open_revision_context(
            &app,
            OpenRevisionIntent {
                shelf_id: shelf_id.clone(),
            },
        )
        .unwrap();
        assert_eq!(context.loader_status, RevisionLoaderStatus::Full);
        assert_eq!(context.revision.annotations.len(), 1);
        let source = std::fs::read(&context.png_path).unwrap();
        let original = std::fs::read(first.png_path.unwrap()).unwrap();
        assert_ne!(source, original);
        let result = commit_revision_native(
            &app,
            runtime.handle(),
            &CommitRevisionIntent {
                shelf_id: shelf_id.clone(),
                annotations: vec![],
                badge_counter: 1,
                active_tool: pixelgrab_contracts::AnnotationTool::Select,
                active_color: AnnotationColor::Red,
                active_stroke: AnnotationStroke::Medium,
                metadata: CacheEntryMetadata::default(),
                to_clipboard: false,
            },
        )
        .unwrap();
        assert_eq!(std::fs::read(result.png_path.unwrap()).unwrap(), source);
        assert_eq!(
            std::fs::read(app.cache().entry(&shelf_id).unwrap().png_path).unwrap(),
            original
        );
        let reopened = open_revision_context(
            &app,
            OpenRevisionIntent {
                shelf_id: result.shelf_id.unwrap(),
            },
        )
        .unwrap();
        assert!(reopened.revision.annotations.is_empty());
        assert_eq!(std::fs::read(reopened.png_path).unwrap(), source);
        app.cache().release_editor_lock(&reopened.shelf_id);
        app.session().cancel_session().unwrap();
    }

    #[test]
    fn unsupported_scene_edits_the_visible_flattened_image_without_losing_baked_annotations() {
        let (app, runtime, _root) = fixture();
        let capture = app
            .session()
            .request_capture(&CaptureRequest {
                format: CaptureFormat::SingleMonitor,
                monitor_id: Some("test".into()),
                region: None,
            })
            .unwrap();
        let annotation = Annotation::rectangle(
            AnnotationId(1),
            PhysicalPoint::new(5, 5),
            PhysicalSize::new(20, 20),
            AnnotationColor::Red,
            AnnotationStroke::Medium,
            0,
        );
        let first = commit_capture(
            &app,
            runtime.handle(),
            &CommitRequest {
                crop: capture.bounds,
                annotations: vec![annotation],
                to_shelf: true,
                to_clipboard: false,
                save_as: false,
            },
        )
        .unwrap();
        let shelf_id = first.shelf_id.unwrap();
        let flat_path = first.png_path.unwrap();
        let flat = std::fs::read(&flat_path).unwrap();
        let mut scene = app.cache().read_revision(&shelf_id).unwrap();
        scene.schema_version += 1;
        let sidecar = std::path::Path::new(&flat_path)
            .parent()
            .unwrap()
            .join("revision.json");
        std::fs::write(sidecar, serde_json::to_vec(&scene).unwrap()).unwrap();
        let context = open_revision_context(
            &app,
            OpenRevisionIntent {
                shelf_id: shelf_id.clone(),
            },
        )
        .unwrap();
        assert_eq!(context.loader_status, RevisionLoaderStatus::FlatFallback);
        assert_eq!(context.png_path, flat_path);
        assert!(context.revision.annotations.is_empty());
        let result = commit_revision_native(
            &app,
            runtime.handle(),
            &CommitRevisionIntent {
                shelf_id,
                annotations: vec![],
                badge_counter: 1,
                active_tool: AnnotationTool::Select,
                active_color: AnnotationColor::Red,
                active_stroke: AnnotationStroke::Medium,
                metadata: CacheEntryMetadata::default(),
                to_clipboard: false,
            },
        )
        .unwrap();
        assert_eq!(std::fs::read(result.png_path.unwrap()).unwrap(), flat);
    }
}
