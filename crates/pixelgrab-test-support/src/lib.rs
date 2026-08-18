//! Deterministic test adapters for PixelGrab.
//!
//! These adapters are the **only** implementation that tests may use for
//! capturing, monitor layout, clock, and filesystem access. They guarantee
//! that no CI run, package build, or test artifact ever contains real screen
//! pixels, real monitor topology, or real user files. See ADR-0004.
//!
//! The same adapters are exposed via the `synthetic` cargo feature in
//! `pixelgrab` so the production binary can be built in a "synthetic" mode
//! for offline demos and the underlying architecture can be exercised without
//! xcap.

#![deny(missing_docs)]

pub mod capture;
pub mod clock;
pub mod fs;
pub mod layout;

pub use capture::{SyntheticCapture, SyntheticFrame};
pub use clock::ControllableClock;
pub use fs::IsolatedFilesystem;
pub use layout::SyntheticMonitorLayout;
