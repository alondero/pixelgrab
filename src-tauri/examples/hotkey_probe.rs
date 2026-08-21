//! Minimal test harness: registers Ctrl+Shift+S via the same
//! global-hotkey crate pixelgrab uses, prints to stderr on every
//! chord press. Runs its own hidden window + message pump, no
//! Tauri, no other dependencies. If pressing Ctrl+Shift+S fires
//! this handler, the OS hotkey layer is fine and any issue is in
//! pixelgrab's wiring. If it does not fire, something on the
//! system (Snipping Tool, NVIDIA overlay, antivirus, etc.) is
//! eating the chord before pixelgrab can see it.
//!
//! Run with: `cargo run --release --example hotkey_probe`

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mgr = GlobalHotKeyManager::new()?;
    let hk = HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyS);
    mgr.register(hk)?;
    eprintln!("hotkey_probe: registered Ctrl+Shift+S (id=0x{:X})", hk.id());

    let count = Arc::new(AtomicUsize::new(0));
    let count_clone = count.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
        let n = count_clone.fetch_add(1, Ordering::SeqCst);
        eprintln!(
            "hotkey_probe: event #{} id=0x{:X} state={:?}",
            n + 1,
            e.id(),
            e.state()
        );
    }));

    eprintln!("hotkey_probe: pumping messages; press Ctrl+Shift+S, Ctrl+C to exit");

    // Run a Win32 message loop on this thread. global-hotkey's
    // WndProc lives on whichever thread created the manager
    // (this thread). GetMessage/DispatchMessage is what pumps
    // WM_HOTKEY into that WndProc.
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, TranslateMessage, MSG,
        };
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        loop {
            let r = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
            if r == 0 || r == -1 {
                break;
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("hotkey_probe: only Windows is supported");
    }
    Ok(())
}
