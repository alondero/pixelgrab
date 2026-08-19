//! Persistent shelf preferences.
//!
//! Tracer 12 introduces user-configurable shelf settings. The
//! [`PreferencesStore`] owns the in-memory state and debounces disk
//! writes; the [`ShelfPreferences`] shape in
//! `pixelgrab_contracts::shelf_preferences` is the wire model.

pub mod debouncer;
pub mod store;

pub use debouncer::Debouncer;
pub use store::{
    default_preferences_root, PreferencesStore, BACKUP_FILENAME, PERSIST_DEBOUNCE, PRIMARY_FILENAME,
};
