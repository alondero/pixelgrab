//! Verifies that the single-instance plugin wiring is present and that the
//! forwarded-intent path reaches the right event channel.
//!
//! The actual "two processes, one wins" test requires a Tauri runtime and is
//! verified manually by launching the packaged binary twice on the same
//! machine. Here we assert the static invariants so a future refactor
//! cannot silently break the wiring.

use pixelgrab_lib::singleton::SINGLE_INSTANCE_EVENT;

/// The forwarded-intent event name is shared between Rust and the
/// frontend. A drift would break the singleton forwarding end-to-end.
#[test]
fn event_channel_constant_matches_frontend() {
    assert_eq!(SINGLE_INSTANCE_EVENT, "pixelgrab://secondary-launch");
}

/// The forward function is publicly accessible from the library crate. This
/// is a presence test - the actual gesture is exercised by the Tauri
/// runtime in the binary and requires manual verification on Windows.
#[test]
fn forward_function_is_publicly_exported() {
    // The function symbol must exist in the public API. If this stops
    // compiling, a refactor has hidden the wiring. Tracer 14
    // threaded the secondary-launch intent through the signature so
    // the type pin keeps the hook closure aligned.
    fn _check() {
        let _f: for<'r> fn(
            &'r tauri::AppHandle<tauri::Wry>,
            pixelgrab_contracts::SecondaryLaunchIntent,
        ) -> () = pixelgrab_lib::singleton::forward_to_existing_instance;
    }
}
