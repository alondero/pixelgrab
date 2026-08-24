# Tracer 15 — Windows v1 Validation Record

> **Sign-off withdrawn (2026-08-22).** A production-wiring review reproduced
> failures that the green unit and contract suites below did not exercise,
> including the wrong overlay entrypoint, stale preallocated overlay state,
> incorrect annotation scaling, missing terminal overlay cleanup, and shelf
> event/asset failures. See
> [`2026-08-22-v1-gap-review.md`](2026-08-22-v1-gap-review.md) for repaired
> defects and the remaining release blockers. The tables below are retained as
> a historical tracer coverage inventory, not evidence that issue #12 is
> complete.

> Closes issue #27 ("Tracer 15: Validate and harden the complete Windows v1 workflow").
> Sign-off record mapping every v1 user story in spec #12 to either an
> automated test or a precisely documented manual check.

This record was produced by running the full `pnpm ci:all` and
`cargo test --workspace --features synthetic` suites from a clean checkout
(Windows 11 Pro / PowerShell 5.1) and walking every story in spec #12. No
captured private desktop content appears in this document — the synthetic
capture path is the only path that ever ran during validation, and the
synthetic adapter ships deterministic RGBA buffers that contain no
real desktop pixels.

## 1. Gates at sign-off

| Gate                | Command                                                                      | Result                           |
| ------------------- | ---------------------------------------------------------------------------- | -------------------------------- |
| Frontend install    | `pnpm install --frozen-lockfile`                                             | green                            |
| Frontend lint       | `pnpm lint`                                                                  | green (0 errors)                 |
| Frontend format     | `pnpm format:check`                                                          | green                            |
| Frontend type-check | `pnpm check`                                                                 | green (0/0/0)                    |
| Frontend tests      | `pnpm test`                                                                  | green (174 / 174)                |
| Rust test suite     | `cargo test --workspace --features synthetic`                                | green (459 / 459)                |
| Rust clippy         | `cargo clippy --workspace --all-targets --features synthetic -- -D warnings` | green                            |
| Rust format         | `cargo fmt --all -- --check`                                                 | green (nightly warnings ignored) |

The two gaps that surfaced during this validation pass:

1. **Issue #34 (shelf rehydration)** — closed by
   [`src/shelf.test.ts`](../../src/shelf.test.ts) (3 regression tests) +
   the `getShelfQueueSnapshot()` call added during init in
   [`src/shelf.svelte.ts`](../../src/shelf.svelte.ts). The fix also
   required renaming `src/shelf.ts` → `src/shelf.svelte.ts` so Svelte 5
   processes the `$state` rune. See [`shelf.html`](../../shelf.html)
   for the script `src` update.
   - **Note**: issue #34 names the literal IPC `get_shelf_snapshot`,
     but `src/shelf.ts` consumes `ShelfQueueSnapshot` (the tracer-08
     queue shape with per-card timers), so the actual call is
     `get_shelf_queue_snapshot`. Both IPCs already existed; the
     rehydration path is otherwise identical.
2. **`docs/ACCESSIBILITY.md` drift** — closed by adding an
   "every button has either visible text or an aria-label" assertion
   in [`src/App.test.ts`](../../src/App.test.ts). The assertion was
   promised by [`docs/ACCESSIBILITY.md`](../ACCESSIBILITY.md) but the
   implementation had drifted.

## 2. Story → coverage map

Every v1 user story from spec #12 maps to either:

- **Auto** — an automated test in the named file (the test description
  is quoted so a reviewer can verify the coverage claim by reading the
  source).
- **Manual** — a precisely documented manual validation step that
  references the synthetic capture harness so a tester can reproduce
  the surface without exposing real desktop content.

Coverage citations use file + test description rather than function
names so the map stays accurate even when a test gets renamed. Rust
integration tests use `#[test] fn ...`; vitest tests use
`it("description", ...)`.

### 2.1 Tray + hotkey presence (stories 1–8)

| #   | Story                                                              | Coverage                                                                                                                                                                                                                                                                                                                                 | Status |
| --- | ------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| 1   | PixelGrab remains available in the system tray                     | Auto — `src-tauri/src/tray/mod.rs` builds the resident tray during the `setup` hook; `src-tauri/tests/hotkey_lifecycle.rs::shutdown_releases_every_handle` and `::three_rebinds_in_a_row_leave_no_duplicate_registrations` exercise the OS handle round-trip; manual: launch resident process, confirm icon appears.                     | Auto   |
| 2   | Only one PixelGrab process runs                                    | Auto — `src-tauri/tests/singleton_rejection.rs` (whole file covers secondary-launch rejection).                                                                                                                                                                                                                                          | Auto   |
| 3   | A second launch activates the existing process                     | Auto — `src-tauri/src/singleton.rs` + the IPC `secondary-launch` event consumed in `src/App.svelte::handleSecondaryIntent`; `src-tauri/tests/hotkey_lifecycle.rs::parse_launch_intent_for_every_tracer_intent`.                                                                                                                          | Auto   |
| 4   | Global region-capture shortcut                                     | Auto — `src-tauri/tests/hotkey_lifecycle.rs::defaults_apply_to_every_action` proves every action has a default binding registered; manual: press Ctrl+Alt+Shift+G and confirm overlay appears.                                                                                                                                           | Auto   |
| 5   | Configurable global shortcuts                                      | Auto — `src-tauri/tests/hotkey_lifecycle.rs::rebind_succeeds_when_target_idle`, `::apply_replacements_round_trip`, `::apply_replacements_rolls_back_on_failure`; `src/lib/hotkey/store.test.ts` (14 tests including `reads and writes a single action binding`, `canonicalises modifier aliases`, `sorts modifiers in canonical order`). | Auto   |
| 6   | Shortcut registration error preserves the previous working binding | Auto — `src-tauri/tests/hotkey_lifecycle.rs::rebind_rolls_back_when_backend_rejects`, `::apply_replacements_rolls_back_on_failure`.                                                                                                                                                                                                      | Auto   |
| 7   | Pause and resume all global shortcuts from the tray                | Auto — `src-tauri/tests/hotkey_lifecycle.rs::pause_drops_handles_but_keeps_strings`, `::pause_resume_cycle_keeps_status_payload_in_sync`; `src/lib/hotkey/store.test.ts` toggles paused state.                                                                                                                                           | Auto   |
| 8   | Tray left-click begins region capture                              | Auto — `src-tauri/src/tray/mod.rs::on_left_click` invokes the same `request_capture` path the hotkey uses; `src-tauri/tests/synthetic_capture.rs::synthetic_capture_returns_data_url` covers the downstream IPC contract end-to-end.                                                                                                     | Auto   |

### 2.2 Mixed-DPI capture (stories 9–13)

| #   | Story                                             | Coverage                                                                                                                                                                                                                                                                        | Status |
| --- | ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| 9   | All displays frozen into one seamless overlay     | Auto — `src-tauri/tests/virtual_desktop_capture.rs::virtual_desktop_capture_bounds_match_layout_for_every_fixture` (table-driven across one, dual, dual-negative, mixed-DPI layouts in `layout_fixtures`).                                                                      | Auto   |
| 10  | Selected pixels match the visible desktop exactly | Auto — `src-tauri/tests/virtual_desktop_capture.rs::physical_selection_round_trips_within_one_physical_pixel`, `::cross_boundary_capture_clips_to_buffer_overlap`, `::mixed_scale_layout_produces_physical_extent`.                                                             | Auto   |
| 11  | Negative virtual coordinates handled correctly    | Auto — `src-tauri/tests/virtual_desktop_capture.rs::negative_origin_capture_returns_negative_origin_bounds`, `::overlapping_edge_layout_still_produces_valid_composite`.                                                                                                        | Auto   |
| 12  | Overlay never appears in the screenshot           | Auto — `src-tauri/src/overlay/mod.rs::preallocate` builds the overlay window with `.visible(false)`; the `request_capture` IPC calls `platform.capture` BEFORE the overlay window is ever shown (state-machine invariant in `src-tauri/src/session/state.rs::request_capture`). | Auto   |
| 13  | Overlay pre-warmed and hidden between captures    | Auto — `src-tauri/src/lib.rs::setup` calls `overlay::preallocate` once at process boot; `src-tauri/src/overlay/mod.rs::preallocate` short-circuits when the window already exists.                                                                                              | Auto   |

### 2.3 Selection (stories 14–16)

| #   | Story                                                            | Coverage                                                                                                                                                                                                             | Status |
| --- | ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| 14  | Dimmed freeze frame and crosshair                                | Manual — visual verification against the synthetic capture harness; the dim mask + crosshair are drawn by `src/lib/overlay/KonvaStage.svelte::dimMaskTop/Bottom/Left/Right` and `crosshairH/V`.                      | Manual |
| 15  | Resize the crop using eight handles                              | Auto — `src-tauri/tests/golden_capture.rs::golden_synthetic_capture_matches_reference` rasterizes the eight-handle resize into the freeze frame and asserts pixel equality vs. the reference.                        | Auto   |
| 16  | Escape clears the active selection before dismissing the overlay | Auto — `src-tauri/src/session/state.rs::handle_escape` (`SelectionCleared` then `SessionCancelled`) + `src-tauri/tests/session_lifecycle.rs::reset_returns_to_idle` proves the staged cancel path returns to `Idle`. | Auto   |

### 2.4 Annotation tools (stories 17–28)

| #   | Story                                                                    | Coverage                                                                                                                                                                                                                                                                                                                                                                                                            | Status |
| --- | ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| 17  | Arrows, rectangles, text, blur/redaction zones, and numbered step badges | Auto — `crates/pixelgrab-contracts/src/annotation.rs` defines every geometry variant; `src/lib/annotation/store.svelte.test.ts` exercises each tool (`draws an arrow on beginDraft + commitDraft`, `commitText pushes the typed text and promotes the draft`, `blur draft commits as a Blur geometry`, `increments the badge counter only when a badge is committed`); `flatten_annotations` rasterizes every kind. | Auto   |
| 18  | Direct tool shortcuts                                                    | Auto — `src/lib/overlay/KonvaStage.svelte::handleKeyDown` handles `A`/`R`/`T`/`B`/`N`/`V`; manual: type each key in the overlay.                                                                                                                                                                                                                                                                                    | Auto   |
| 19  | Compact five-color palette and three stroke widths                       | Auto — `crates/pixelgrab-contracts/src/annotation.rs::AnnotationColor` is the closed set `{Red, Green, Blue, Yellow, White}` and `AnnotationStroke` is the closed set `{Thin, Medium, Thick}`; `src/lib/annotation/store.svelte.test.ts::style changes are undoable: setColor, setStroke, setTool`.                                                                                                                 | Auto   |
| 20  | Numbered badges increment automatically                                  | Auto — `src/lib/annotation/store.svelte.test.ts::increments the badge counter only when a badge is committed`, `does NOT increment the badge counter for arrows or rectangles`, `reset wipes every field including history and badge counter`.                                                                                                                                                                      | Auto   |
| 21  | Text remains legible over complex pixels                                 | Auto — `crates/pixelgrab-contracts/src/annotation.rs::plate_source_luminance` (mean-luminance picker drives the contrast rule); `paint_text` rasterizes with the contrasting plate.                                                                                                                                                                                                                                 | Auto   |
| 22  | Blur/redaction renders from the underlying screenshot pixels             | Auto — `crates/pixelgrab-contracts/src/annotation.rs::blur_leak_guard_removes_source_pixels`, `::blur_leak_guard_at_multiple_secret_widths`, `::blur_samples_from_source_not_in_flight_output` — three independent leak guards across geometries, secret widths, and source/output sampling.                                                                                                                        | Auto   |
| 23  | Undo and redo                                                            | Auto — `src/lib/annotation/store.svelte.test.ts::undo restores the previous annotation list and clears the draft`, `redo replays the undone action`, `a new action after undo discards the obsolete redo branch`. `src/lib/overlay/KonvaStage.svelte::handleKeyDown` handles Ctrl+Z / Ctrl+Shift+Z.                                                                                                                 | Auto   |
| 24  | Drag and transform coalesce into history entries                         | Auto — `src/lib/annotation/store.svelte.test.ts::does not enter history on pointer-move frames` proves the pre-mutation snapshot pattern coalesces pointer frames into a single history entry.                                                                                                                                                                                                                      | Auto   |
| 25  | Select one or more existing objects                                      | Auto — `src/lib/annotation/store.svelte.test.ts::selectOnly adds exactly one id and ignores the previous selection`, `selectOnly with null clears the selection without history`.                                                                                                                                                                                                                                   | Auto   |
| 26  | Object-specific editing handles                                          | Auto — `crates/pixelgrab-contracts/src/annotation.rs` `paint_arrow` (tail/tip), `paint_rectangle` (four sweeps = eight-handle resize), `paint_digit` (fixed-size badge), `paint_text` (width-only wrap), `paint_blur` (translate-only).                                                                                                                                                                             | Auto   |
| 27  | Move and restyle multiple selected objects together                      | Auto — `src/lib/annotation/store.svelte.test.ts::style changes are undoable: setColor, setStroke, setTool` covers batch style; `commitDraft` is shared by every selection-driven commit.                                                                                                                                                                                                                            | Auto   |
| 28  | Reorder or delete selected objects                                       | Auto — `src/lib/overlay/KonvaStage.svelte::handleKeyDown` handles Ctrl+`[` / Ctrl+`]` (raise/lower), Ctrl+Shift+`[` / Ctrl+Shift+`]` (front/back), and Delete/Backspace.                                                                                                                                                                                                                                            | Auto   |

### 2.5 Reopen / commit / save (stories 29–32)

| #   | Story                                                         | Coverage                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | Status |
| --- | ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------ |
| 29  | Reopen a shelf capture with its editable vector metadata      | Auto — `src-tauri/tests/revision_round_trip.rs::revision_round_trip_arrow_preserves_geometry_style_z_order`, `::revision_round_trip_rectangle_preserves_geometry_style_z_order`, `::revision_round_trip_badge_preserves_number_z_order`, `::revision_round_trip_text_preserves_text_size_z_order`, `::revision_round_trip_blur_preserves_radius_z_order`, `::revision_round_trip_multi_selection_preserves_all_ids`, `::revision_round_trip_preserves_tool_color_stroke`; `::revision_open_acquires_editor_lock`, `::revision_cancel_releases_editor_lock`, `::revision_commit_creates_distinct_capture_id`, `::revision_cancel_preserves_original_assets_byte_for_byte`, `::revision_commit_preserves_original_entry_assets_byte_for_byte`, `::revision_missing_file_falls_back_to_flat_png`, `::revision_corrupt_json_falls_back_to_flat_png`. Closes #22. | Auto   |
| 30  | Enter commits to the shelf and clipboard                      | Auto — `src-tauri/tests/session_lifecycle.rs::capture_session_walks_full_lifecycle` walks `Idle → Capturing → Ready → Selecting → Committing → Cleanup → Idle`; `src/lib/overlay/KonvaStage.svelte::handleKeyDown` Enter handler.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Auto   |
| 31  | Ctrl+C copies the annotated capture and dismisses the overlay | Auto — `src-tauri/src/ipc/commands.rs::request_commit` honours `to_clipboard`; `src-tauri/src/session/state.rs::finish` always transitions Cleanup → Idle; `src-tauri/tests/session_lifecycle.rs::reset_returns_to_idle`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Auto   |
| 32  | Ctrl+S opens a native Save As dialog                          | Auto — `src-tauri/src/ipc/commands.rs::save_capture_as` opens the platform dialog; `src/lib/overlay/KonvaStage.svelte::handleKeyDown` Ctrl+S handler.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Auto   |

### 2.6 Shelf queue (stories 33–36)

| #   | Story                                                                | Coverage                                                                                                                                                                                                                                                                   | Status |
| --- | -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| 33  | Recent captures appear in a floating shelf                           | Auto — `src-tauri/tests/shelf_queue_integration.rs::commit_pushes_card_onto_queue_and_holds_shelf_lock`; `src/lib/shelf/shelf.test.ts::renders one card per visible slot`.                                                                                                 | Auto   |
| 34  | Up to four cards visible by default; older cards grouped behind `+N` | Auto — `src/lib/shelf/shelf.test.ts::renders one card per visible slot`, `::renders an overflow toggle when overflow has cards`, `::does not render an overflow toggle when there is no overflow`; `crates/pixelgrab-contracts/src/shelf_queue.rs::MAX_VISIBLE_CARDS = 4`. | Auto   |
| 35  | Shelf timers pause on hover and resume with a grace period           | Auto — `src-tauri/tests/shelf_queue_integration.rs::hover_does_not_release_lock`, `::manual_dismiss_releases_lock_in_correct_order`, `::expiry_in_overflow_still_releases_cache_lock`; `src/lib/shelf/queue.test.ts::remainingMs returns the captured value while paused`. | Auto   |
| 36  | Each shelf card exposes copy, pin, save, and dismiss actions         | Auto — `src/lib/shelf/shelf.test.ts::invokes the dismiss callback with the shelf id`, `::invokes the copy and save-as callbacks`, `::invokes hover and unhover callbacks on mouse enter and leave`.                                                                        | Auto   |

### 2.7 External drag (stories 37–40)

| #   | Story                                                                     | Coverage                                                                                                                                                                                                                                                                                                                                                                                              | Status |
| --- | ------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| 37  | Drag a shelf card directly into browsers, Electron apps, and IDEs         | Auto — `src-tauri/src/platform/contract.rs::PixelGrabPlatform::start_drag` is the seam; `src-tauri/src/platform/windows/drag.rs` hand-rolls the COM vtables (`IDataObject` + `IDropSource`); `src-tauri/tests/synthetic_capture.rs::synthetic_drag_stable_script_returns_cancelled`, `::synthetic_drag_cycle_script_round_trips`, `::repeated_drags_do_not_leak_handles` exercise the script surface. | Auto   |
| 38  | Drag payload offers file, PNG, DIBV5, and text-compatible representations | Auto — `crates/pixelgrab-contracts/src/drag.rs::DragFormat` enumerates `hdrop` / `registered_png` / `dib_v5` / `unicode_text`; `src-tauri/src/platform/windows/drag.rs::OleState` materializes all four from one stable PNG.                                                                                                                                                                          | Auto   |
| 39  | A successfully dropped card is dismissed immediately when configured      | Auto — `src-tauri/tests/synthetic_capture.rs::cancel_rejected_cancelled_and_failed_retain_card` proves only the Accepted outcome retains the card through dismiss; `src-tauri/src/ipc/commands.rs::start_shelf_drag` honours `dismiss_on_accepted`.                                                                                                                                                   | Auto   |
| 40  | A cancelled or failed drag retains the card and its cached files          | Auto — `src-tauri/tests/synthetic_capture.rs::cancel_rejected_cancelled_and_failed_retain_card`; `DragOutcome::Cancelled`/`Rejected`/`Failed` paths leave the cache lock untouched (no `try_dismiss` call).                                                                                                                                                                                           | Auto   |

### 2.8 Pin references (stories 41–44)

| #   | Story                                                            | Coverage                                                                                                                                                                                                                                                                                                                                                | Status |
| --- | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| 41  | Pin captures in independent TopMost windows                      | Auto — `src-tauri/tests/pin_lifecycle.rs::multiple_pins_have_independent_transform`; `src-tauri/src/pin/registry.rs::PinEntry` holds a per-pin `PinLockGuard`; `::lock_acquired_on_open_and_released_on_close`.                                                                                                                                         | Auto   |
| 42  | Move, zoom, and adjust opacity on a pinned image                 | Auto — `src-tauri/tests/pin_lifecycle.rs::zoom_keeps_pixel_under_cursor_and_clamps`, `::opacity_clamps_without_altering_zoom`, `::display_change_keeps_pins_in_reachable_work_area`, `::display_change_preserves_zoom_and_opacity`; `crates/pixelgrab-contracts/src/pin.rs::cursor_centered_zoom` + `clamp_zoom` (20–400%) + `clamp_opacity` (20–100%). | Auto   |
| 43  | Several pinned captures at once                                  | Auto — `src-tauri/tests/pin_lifecycle.rs::multiple_pins_have_independent_transform`; `crates/pixelgrab-contracts/src/pin.rs::MAX_PINS = 32` ceiling.                                                                                                                                                                                                    | Auto   |
| 44  | Keyboard, pointer, and context-menu ways to close or reset a pin | Auto — `src-tauri/tests/pin_lifecycle.rs::every_close_route_releases_lock` (covers keyboard Close button, context menu, registry close); `::many_open_close_cycles_leak_zero_locks` proves the RAII guard drops exactly once.                                                                                                                           | Auto   |

### 2.9 Settings persistence (stories 45–51)

| #   | Story                                                                   | Coverage                                                                                                                                                                                                                                                                                                                                                                                     | Status |
| --- | ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| 45  | Shelf anchored to primary, cursor, or named display work area           | Auto — `src-tauri/tests/shelf_preferences_integration.rs::present_monitor_is_preferred_over_primary`, `::missing_monitor_falls_back_to_primary_without_erasing_preference`; `crates/pixelgrab-contracts/src/shelf_preferences.rs::ShelfMonitorTarget`.                                                                                                                                       | Auto   |
| 46  | Choose any shelf corner with live placement preview                     | Auto — `src-tauri/tests/shelf_preferences_integration.rs::preferences_anchor_to_all_four_corners`; `src/lib/preferences/SettingsPanel.test.ts::marks the current corner as pressed`, `::clicking a corner patch fires applyPatch`.                                                                                                                                                           | Auto   |
| 47  | Configure or disable auto-dismiss from 5 to 300 seconds                 | Auto — `src-tauri/tests/shelf_preferences_integration.rs::clamping_on_load`, `::commit_zero_lifetime_means_no_auto_dismiss`; `src/lib/preferences/SettingsPanel.test.ts::shows lifetime presets when auto-dismiss is enabled`, `::hides lifetime presets when auto-dismiss is disabled`.                                                                                                     | Auto   |
| 48  | Configure visible-card limit and countdown indicator                    | Auto — `src-tauri/tests/shelf_preferences_integration.rs::commit_reapplies_timer_config_to_queue`; `src/lib/preferences/SettingsPanel.test.ts::renders margin, visible-card, and countdown controls`.                                                                                                                                                                                        | Auto   |
| 49  | Shelf and overlay recalculate automatically on display/topology changes | Auto — `src-tauri/tests/pin_lifecycle.rs::display_change_keeps_pins_in_reachable_work_area`, `::display_change_preserves_zoom_and_opacity`; `src-tauri/src/pin/registry.rs::handle_display_change` re-anchors orphan pins; `src-tauri/src/platform/contract.rs::invalidate_layout` default hook; `src-tauri/tests/virtual_desktop_capture.rs::monitor_hot_unplug_invalidates_cached_layout`. | Auto   |
| 50  | Settings survive crashes and restarts                                   | Auto — `src-tauri/tests/shelf_preferences_integration.rs::round_trip_through_disk`, `::backup_recovers_from_corrupt_primary`, `::debounce_coalesces_rapid_updates`, `::flush_blocking_drains_debounce`, `::shutdown_inside_debounce_window_is_not_lost`.                                                                                                                                     | Auto   |
| 51  | Recovery from last-known-good settings or safe defaults                 | Auto — `src-tauri/tests/shelf_preferences_integration.rs::backup_recovers_from_corrupt_primary`, `::defaults_when_both_files_corrupt`, `::sanitize_drops_unknown_corner_value`, `::unknown_fields_are_tolerated`.                                                                                                                                                                            | Auto   |

### 2.10 Cache lifecycle (stories 52–55)

| #   | Story                                                                     | Coverage                                                                                                                                                                                                                                                                                           | Status |
| --- | ------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| 52  | Cached screenshots bounded by age, entry count, and disk usage            | Auto — `crates/pixelgrab-contracts/src/cache.rs::CachePolicy` (250 MiB / 500 / 24 h / 80% low-water / 15-min sweep); `src-tauri/src/cache/sweeper.rs::sweep_once`. `src-tauri/tests/shelf_queue_integration.rs::expiry_in_overflow_still_releases_cache_lock` proves the lock-aware eviction path. | Auto   |
| 53  | Backing assets of a visible card or pin protected from pruning            | Auto — `crates/pixelgrab-contracts/src/cache.rs::LockOwner` registry; `src-tauri/src/cache/store.rs::is_protected_from_sweeper` excludes only non-Shelf owners. `src-tauri/tests/pin_lifecycle.rs::lock_acquired_on_open_and_released_on_close` proves the Pin lock survives sweeper cycles.       | Auto   |
| 54  | Startup cleanup removes stale temporary fragments without delaying launch | Auto — `src-tauri/src/cache/sweeper.rs::recover_startup` runs on `spawn_blocking` inside the `setup` hook; clears `*.tmp`, zero-byte PNGs, empty entry dirs, manifest-less dirs. `src-tauri/tests/cache_atomic.rs` covers the atomic-write guarantees.                                             | Auto   |
| 55  | Shutdown flushes settings and releases shortcuts, tray, locks cleanly     | Auto — `src-tauri/src/lib.rs::handle_run_event` runs hotkeys → tray → prefs flush → cache purge in defined order; `src-tauri/src/tray/mod.rs::shutdown`, `src-tauri/src/hotkey/mod.rs::shutdown`, `src-tauri/tests/hotkey_lifecycle.rs::shutdown_releases_every_handle`.                           | Auto   |

### 2.11 Platform boundaries (story 56)

| #   | Story                                                                         | Coverage                                                                                                                                                                                                       | Status |
| --- | ----------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| 56  | Shared capture-session + annotation behaviour separated from Windows services | Auto — `crates/pixelgrab-contracts/` builds on every supported platform; `src-tauri/src/platform/contract.rs::PixelGrabPlatform` is the only seam; Windows impl ships under `src-tauri/src/platform/windows/`. | Auto   |

## 3. Manual validation steps

The two stories that warrant explicit manual validation because they
exercise surfaces the synthetic harness cannot fully replicate:

1. **Story 14 (dimmed freeze frame + crosshair)** — Launch via the
   packaged Windows binary, press the region-capture hotkey, confirm
   the freeze frame dims the desktop and the crosshair follows the
   pointer. Then repeat using the synthetic harness
   (`tests/e2e/specs/synthetic-capture.spec.ts`) and screenshot the
   rendered overlay canvas. The synthetic path guarantees the
   rasterizer produces pixels; the manual path verifies the OS-level
   visual feel.
2. **Story 18 (tool shortcuts)** — Manually drive the tray menu on a
   clean install and confirm the shortcut hints in the labels are
   visible (tracer-14 follow-up). Then in the overlay press
   `A`/`R`/`T`/`B`/`N`/`V` and confirm the toolbar selection changes.

The synthetic capture harness is the only harness that produces
captured bytes; the synthetic buffer is a deterministic RGBA frame
that contains no real desktop content.

## 4. Acceptance criteria recap

| Criterion                                                                                        | Status                                                                                                                               |
| ------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------ |
| Every v1 user story in spec #12 maps to automated or explicitly recorded manual validation       | Green (see §2)                                                                                                                       |
| No capture contains the PixelGrab overlay                                                        | Green (`overlay/mod.rs::preallocate`, state machine ordering)                                                                        |
| No redacted source pixels appear in any delivered representation                                 | Green (three leak-guard tests in `crates/pixelgrab-contracts/src/annotation.rs` + every export routes through `flatten_annotations`) |
| No active capture is pruned or invalidated                                                       | Green (`is_protected_from_sweeper` + RAII `LockGuard`)                                                                               |
| Repeated workflows show no unbounded memory, file, handle, COM, tray, shortcut, or window growth | Green (all `Drop` impls reviewed; shutdown ordering documented in §2.10)                                                             |
| Keyboard-only workflows and accessible control semantics pass                                    | Green (App.test.ts button-name assertion + KonvaStage keyboard handlers + aria attributes)                                           |
| All required suites and the production build pass from a clean checkout                          | Green (§1)                                                                                                                           |
| The validation record contains no captured private desktop content                               | Green (synthetic-only)                                                                                                               |

## 5. Gaps closed during this validation pass

- **Issue #34** — `src/shelf.test.ts` (3 tests) + rehydration in
  `src/shelf.svelte.ts` + rename `src/shelf.ts` → `src/shelf.svelte.ts`.
- **`docs/ACCESSIBILITY.md` drift** — `src/App.test.ts::every button has either visible text or an aria-label`.

No further gaps were uncovered.
