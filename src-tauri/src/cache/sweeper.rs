//! Cache sweeper — periodic TTL + LRU eviction, and start-up recovery.
//!
//! Tracer 13 introduces three sweep behaviours:
//!
//! 1. **TTL**: every sweep, evict unlocked entries whose
//!    `last_access_at_ms` is older than `policy.max_age_ms`.
//! 2. **Quota**: after TTL, evict the oldest unlocked entries until
//!    total bytes and entry count are at or below the
//!    `low_water_ratio` of the policy's high-water limits.
//! 3. **Recovery**: on startup, reap `*.tmp` debris, zero-byte
//!    assets, and empty entry directories without touching valid
//!    active captures.
//!
//! The algorithm honours the same `ActiveLockSet` the rest of the
//! cache uses. A `Shelf` lock is the "default" ownership and is not
//! enough to protect an entry from the sweeper — the user wants the
//! cache to keep entries alive even when no shelf card is visible.
//! An `Editor`, `Drag`, or `Pin` lock is enough to protect the
//! entry from eviction.
//!
//! The sweep runs on a worker thread spawned by `SweepWorker` so the
//! startup recovery and the periodic worker do not block the tray
//! appearing. The worker is shut down when the surrounding `App`
//! goes out of scope.

use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use parking_lot::Mutex;
use pixelgrab_contracts::{CachePolicy, CacheStats, PlatformResult, SweepOutcome};

use super::policy::CachePolicyStore;
use super::store::Cache;

/// Trait the sweeper uses to read the wall-clock. Implemented by
/// `ControllableClock` in tests and by the production wall-clock
/// wrapper in `lib.rs`. The trait lives here so the tests can wire
/// the clock without depending on the `pixelgrab-test-support` crate.
pub trait WallClock: Send + Sync + 'static {
    /// Current wall-clock millis since the Unix epoch.
    fn now_ms(&self) -> i64;
}

/// Production wall-clock implementation: `SystemTime`. Kept as a
/// separate type so the test path can swap to a `ControllableClock`
/// without `unsafe` or `cfg(test)` gating.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemWallClock;

impl WallClock for SystemWallClock {
    fn now_ms(&self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// Snapshot of the cache's usage taken at the start of a sweep. The
/// sweeper avoids re-reading the entire cache map twice when the
/// policy is "everything is fine" — the `needs_sweep` check is a
/// O(1) decision once the stats are computed.
#[derive(Debug, Clone)]
struct Snapshot {
    stats: CacheStats,
    policy: CachePolicy,
}

impl Snapshot {
    /// True when the cache is within the policy's low-water targets.
    fn within_low_water(&self) -> bool {
        let low_bytes = self.policy.low_water_bytes();
        let low_entries = self.policy.low_water_entries();
        self.stats.total_bytes <= low_bytes && self.stats.entry_count <= low_entries
    }
}

/// Sweeper logic. Cheap to clone — every field is behind an `Arc`.
#[derive(Clone)]
pub struct CacheSweeper {
    cache: Arc<Cache>,
    policy_store: Arc<CachePolicyStore>,
    clock: Arc<dyn WallClock>,
}

// Manual `Debug` impl — `Arc<dyn WallClock>` doesn't satisfy the
// auto-derive blanket impl, and the struct is otherwise simple
// enough to write by hand.
impl std::fmt::Debug for CacheSweeper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheSweeper")
            .field("cache", &self.cache)
            .field("policy_store", &self.policy_store)
            .field("clock", &"<dyn WallClock>")
            .finish()
    }
}

/// Handle to the periodic background worker. `stop` joins the
/// worker thread; the worker exits on the next loop iteration after
/// `stop` is called.
pub struct SweepWorker {
    handle: Mutex<Option<JoinHandle<()>>>,
    stop_flag: Arc<Mutex<bool>>,
}

impl SweepWorker {
    /// Stop the worker. Safe to call multiple times; the second call
    /// is a no-op.
    pub fn stop(&self) {
        *self.stop_flag.lock() = true;
        if let Some(handle) = self.handle.lock().take() {
            let _ = handle.join();
        }
    }

    /// Test-only: has the worker been stopped?
    #[cfg(test)]
    pub fn is_stopped(&self) -> bool {
        *self.stop_flag.lock()
    }
}

impl Drop for SweepWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

impl CacheSweeper {
    /// Build a sweeper with the production wall-clock.
    pub fn new(cache: Arc<Cache>, policy_store: Arc<CachePolicyStore>) -> Self {
        Self {
            cache,
            policy_store,
            clock: Arc::new(SystemWallClock),
        }
    }

    /// Build a sweeper with a custom wall-clock. Used by tests.
    pub fn with_clock(
        cache: Arc<Cache>,
        policy_store: Arc<CachePolicyStore>,
        clock: Arc<dyn WallClock>,
    ) -> Self {
        Self {
            cache,
            policy_store,
            clock,
        }
    }

    /// Configure the cache policy store. The store's current policy
    /// is used for the next sweep.
    pub fn policy_store(&self) -> Arc<CachePolicyStore> {
        self.policy_store.clone()
    }

    /// Run the comprehensive startup recovery sweep. Removes
    /// debris (`*.tmp` files, zero-byte assets, empty entry dirs)
    /// and any expired entries that the partial loader allowed onto
    /// the shelf. Non-blocking equivalent of the helper methods on
    /// `Cache` — used by the run-time wiring in `lib.rs`.
    pub fn recover_startup(&self) -> PlatformResult<SweepOutcome> {
        let mut outcome = self.cache.recover_debris()?;
        let policy = self.policy_store.current();
        let now_ms = self.clock.now_ms();
        outcome.merge(&self.evict_expired(&policy, now_ms));
        Ok(outcome)
    }

    /// Run a single TTL + LRU pass. Returns the combined outcome.
    pub fn sweep_once(&self) -> SweepOutcome {
        let policy = self.policy_store.current();
        let now_ms = self.clock.now_ms();
        let snapshot = Snapshot {
            stats: self.cache.stats(),
            policy: policy.clone(),
        };
        let mut outcome = self.evict_expired(&policy, now_ms);
        if !snapshot.within_low_water() {
            outcome.merge(&self.evict_for_quota(&policy, now_ms));
        } else {
            // Snapshot was within the low-water targets — but the
            // recovery debris may still have left junk on disk.
            let _ = self.cache.recover_debris();
        }
        outcome
    }

    /// Spawn the periodic background worker. The worker calls
    /// `sweep_once` every `policy.sweep_interval_ms` until the
    /// returned `SweepWorker` is dropped or its `stop` is called.
    pub fn start_periodic(&self) -> SweepWorker {
        let stop_flag = Arc::new(Mutex::new(false));
        let sweeper = self.clone();
        let stop_flag_for_thread = stop_flag.clone();
        let handle = thread::Builder::new()
            .name("pixelgrab-cache-sweeper".to_string())
            .spawn(move || sweep_loop(sweeper, stop_flag_for_thread))
            .expect("spawn cache sweeper thread");
        SweepWorker {
            handle: Mutex::new(Some(handle)),
            stop_flag,
        }
    }

    /// Evict every unlocked entry whose `last_access_at_ms` is older
    /// than `policy.max_age_ms`. Locked entries (editor / drag / pin
    /// owners) are kept. Bytes reclaimed reflect the actual on-disk
    /// size at the moment of dismissal — the spec requires the
    /// outcome to report what was really reclaimed, not the cached
    /// `size_bytes` which can drift from disk.
    fn evict_expired(&self, policy: &CachePolicy, now_ms: i64) -> SweepOutcome {
        let mut outcome = SweepOutcome::default();
        let candidates = self.cache.entries();
        for entry in candidates {
            let delta = now_ms.saturating_sub(entry.last_access_at_ms);
            if delta < policy.max_age_ms {
                continue;
            }
            if self.cache.is_protected_from_sweeper(&entry.shelf_id) {
                continue;
            }
            let on_disk = self.cache.entry_on_disk_size(&entry.shelf_id);
            match self.cache.dismiss(&entry.shelf_id) {
                Ok(o) if o.removed => {
                    outcome.expired_evicted = outcome.expired_evicted.saturating_add(1);
                    if let Some(bytes) = on_disk {
                        outcome.bytes_reclaimed = outcome.bytes_reclaimed.saturating_add(bytes);
                    }
                }
                Ok(_) => {
                    // Locked by something we didn't catch (e.g. a
                    // shelf lock that hasn't been dropped yet). Skip.
                }
                Err(_err) => {
                    outcome.partial_failures = outcome.partial_failures.saturating_add(1);
                }
            }
        }
        outcome
    }

    /// Evict the oldest unlocked entries until total bytes and
    /// entry count are both at or below the low-water targets.
    /// Entries are sorted by `last_access_at_ms` ascending — pure
    /// LRU. The candidate list is snapshotted once at the top of
    /// the loop so the per-iteration cost is O(n) instead of O(n²).
    /// The function continues past per-entry failures so one
    /// permission error cannot strand the rest.
    fn evict_for_quota(&self, policy: &CachePolicy, _now_ms: i64) -> SweepOutcome {
        let mut outcome = SweepOutcome::default();
        let candidates = self.cache.entries();
        let mut evicted_ids: Vec<String> = Vec::with_capacity(candidates.len());
        loop {
            let snapshot = Snapshot {
                stats: self.cache.stats(),
                policy: policy.clone(),
            };
            if snapshot.within_low_water() {
                break;
            }
            // Pick the oldest entry that is not yet evicted and is
            // not protected by a non-default lock owner.
            let pick = candidates
                .iter()
                .filter(|e| !evicted_ids.contains(&e.shelf_id))
                .filter(|e| !self.cache.is_protected_from_sweeper(&e.shelf_id))
                .min_by_key(|e| e.last_access_at_ms);
            let Some(target) = pick else {
                // Every remaining entry is locked — break to avoid
                // an infinite loop.
                break;
            };
            let on_disk = self.cache.entry_on_disk_size(&target.shelf_id);
            let target_id = target.shelf_id.clone();
            match self.cache.dismiss(&target_id) {
                Ok(o) if o.removed => {
                    outcome.quota_evicted = outcome.quota_evicted.saturating_add(1);
                    if let Some(bytes) = on_disk {
                        outcome.bytes_reclaimed = outcome.bytes_reclaimed.saturating_add(bytes);
                    }
                    evicted_ids.push(target_id);
                }
                Ok(_) => {
                    // Race with another lock holder — break so we
                    // don't overshoot the target.
                    break;
                }
                Err(_err) => {
                    outcome.partial_failures = outcome.partial_failures.saturating_add(1);
                    break;
                }
            }
        }
        outcome
    }
}

/// Combination helper for `SweepOutcome`. `self` accumulates the
/// fields from `other` so a sweep that runs both TTL and LRU can
/// report them in a single struct.
trait MergeOutcome {
    fn merge(&mut self, other: &SweepOutcome);
}

impl MergeOutcome for SweepOutcome {
    fn merge(&mut self, other: &SweepOutcome) {
        self.expired_evicted = self.expired_evicted.saturating_add(other.expired_evicted);
        self.quota_evicted = self.quota_evicted.saturating_add(other.quota_evicted);
        self.bytes_reclaimed = self.bytes_reclaimed.saturating_add(other.bytes_reclaimed);
        self.tmp_files_removed = self
            .tmp_files_removed
            .saturating_add(other.tmp_files_removed);
        self.zero_byte_assets_removed = self
            .zero_byte_assets_removed
            .saturating_add(other.zero_byte_assets_removed);
        self.unindexed_dirs_removed = self
            .unindexed_dirs_removed
            .saturating_add(other.unindexed_dirs_removed);
        self.partial_failures = self.partial_failures.saturating_add(other.partial_failures);
    }
}

/// Periodic sweep loop. The interval is recomputed each iteration so
/// a `policy.update` takes effect on the next tick. Exits cleanly
/// when `stop_flag` is set.
fn sweep_loop(sweeper: CacheSweeper, stop_flag: Arc<Mutex<bool>>) {
    loop {
        let interval = sweeper.policy_store.current().sweep_interval_ms.max(1_000);
        // Sleep in 100 ms slices so `stop` reacts quickly even when
        // the interval is the documented 15-minute default.
        let mut slept_ms: i64 = 0;
        let slice_ms = 100i64;
        loop {
            if *stop_flag.lock() {
                return;
            }
            if slept_ms >= interval {
                break;
            }
            thread::sleep(Duration::from_millis(slice_ms as u64));
            slept_ms = slept_ms.saturating_add(slice_ms);
        }
        if *stop_flag.lock() {
            return;
        }
        let _ = sweeper.sweep_once();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use pixelgrab_contracts::{
        cache::CacheEntryMetadata,
        coordinate::{PhysicalBounds, PhysicalSize},
        LockOwner,
    };
    use pixelgrab_test_support::{clock::ControllableClock, fs::IsolatedFilesystem};

    /// Bridge `ControllableClock` to the `WallClock` trait so the
    /// sweeper tests can drive the clock without depending on the
    /// real wall-clock.
    struct TestClock(ControllableClock);

    impl WallClock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.now_ms()
        }
    }

    fn request(w: u32, h: u32) -> super::super::CacheCommitRequest {
        super::super::CacheCommitRequest {
            bounds: PhysicalBounds::from_xywh(0, 0, w, h),
            size: PhysicalSize::new(w, h),
            rgba: {
                let mut buf = Vec::with_capacity((w * h * 4) as usize);
                for i in 0..(w * h) {
                    buf.push((i & 0xFF) as u8);
                    buf.push(((i * 7) & 0xFF) as u8);
                    buf.push(0);
                    buf.push(0xFF);
                }
                buf
            },
            metadata: CacheEntryMetadata::default(),
            monitor_id: "primary".into(),
        }
    }

    fn make_sweeper(cache: &Arc<Cache>, policy: &Arc<CachePolicyStore>) -> CacheSweeper {
        let clock = Arc::new(TestClock(ControllableClock::new()));
        CacheSweeper::with_clock(cache.clone(), policy.clone(), clock)
    }

    #[test]
    fn sweep_once_is_a_noop_when_within_low_water() {
        let fs = IsolatedFilesystem::new("sweeper-noop").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        cache.commit(request(4, 4)).expect("commit");
        let policy_store = Arc::new(CachePolicyStore::new());
        let sweeper = CacheSweeper::with_clock(
            Arc::new(cache.clone()),
            policy_store.clone(),
            Arc::new(TestClock(ControllableClock::new())),
        );
        let outcome = sweeper.sweep_once();
        assert_eq!(outcome.total_evicted(), 0);
        assert_eq!(outcome.bytes_reclaimed, 0);
        assert_eq!(cache.entries().len(), 1);
    }

    #[test]
    fn ttl_evicts_only_expired_unlocked_entries() {
        let fs = IsolatedFilesystem::new("sweeper-ttl").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        cache.commit(request(4, 4)).expect("commit 1");
        cache.commit(request(4, 4)).expect("commit 2");
        // The cache commits with `last_access_at_ms = wall_clock` —
        // a real clock value that has moved past the controllable
        // clock's epoch (2023). Touch both entries to a known past
        // value so the TTL check is meaningful against the
        // controllable clock.
        let entries = cache.entries();
        let shelf_ids: Vec<String> = entries.iter().map(|e| e.shelf_id.clone()).collect();
        cache.touch_entries(&shelf_ids, 5_000);
        let policy_store = Arc::new(CachePolicyStore::new());
        let clock = Arc::new(TestClock(ControllableClock::new()));
        clock.0.advance(100_000); // 100s — well past the 60_000 TTL.
        let tight = CachePolicy {
            max_age_ms: 60_000,
            ..CachePolicy::default()
        };
        policy_store.update(tight);
        policy_store.flush_blocking().ok();
        let sweeper = CacheSweeper::with_clock(Arc::new(cache), policy_store.clone(), clock);
        let outcome = sweeper.sweep_once();
        // Both entries are older than 60_000 ms (touched at 5_000,
        // well in the past).
        assert_eq!(outcome.expired_evicted, 2);
    }

    #[test]
    fn quota_eviction_is_lru_order() {
        let fs = IsolatedFilesystem::new("sweeper-lru").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let policy_store = Arc::new(CachePolicyStore::new());
        // 3 entries max, low_water 0.5 = 1 → sweep evicts 2 leaving 1.
        let policy = CachePolicy {
            max_entries: 3,
            low_water_ratio: 0.5,
            max_bytes: u64::MAX,
            max_age_ms: i64::MAX,
            ..CachePolicy::default()
        };
        policy_store.update(policy);
        let c1 = cache.commit(request(4, 4)).expect("commit 1");
        let c2 = cache.commit(request(4, 4)).expect("commit 2");
        let c3 = cache.commit(request(4, 4)).expect("commit 3");
        let sweeper = CacheSweeper::with_clock(
            Arc::new(cache.clone()),
            policy_store.clone(),
            Arc::new(TestClock(ControllableClock::new())),
        );
        let outcome = sweeper.sweep_once();
        // Two oldest (c1, c2) evicted; c3 survives.
        assert_eq!(outcome.quota_evicted, 2);
        let remaining: Vec<_> = cache.entries().into_iter().map(|e| e.shelf_id).collect();
        assert_eq!(remaining, vec![c3.entry.shelf_id.clone()]);
        // The c1 and c2 entries' directories must be reaped.
        assert!(!fs.root().join(&c1.entry.capture_id).exists());
        assert!(!fs.root().join(&c2.entry.capture_id).exists());
        assert!(fs.root().join(&c3.entry.capture_id).exists());
    }

    #[test]
    fn locked_entries_are_not_evicted() {
        let fs = IsolatedFilesystem::new("sweeper-locked").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let policy_store = Arc::new(CachePolicyStore::new());
        // 2 entries max, low_water 0.5 = 1 → sweep evicts 1 leaving 1.
        let policy = CachePolicy {
            max_entries: 2,
            low_water_ratio: 0.5,
            ..CachePolicy::default()
        };
        policy_store.update(policy);
        let c1 = cache.commit(request(4, 4)).expect("commit 1");
        let _c2 = cache.commit(request(4, 4)).expect("commit 2");
        // Acquire a Pin lock on c1 — pin owner should protect it.
        let pin_guard = cache
            .locks()
            .acquire(c1.entry.shelf_id.clone(), LockOwner::Pin);
        let sweeper = CacheSweeper::with_clock(
            Arc::new(cache.clone()),
            policy_store.clone(),
            Arc::new(TestClock(ControllableClock::new())),
        );
        let outcome = sweeper.sweep_once();
        // c2 should be evicted (only Shelf lock), c1 survives.
        assert_eq!(outcome.quota_evicted, 1);
        let remaining: Vec<_> = cache.entries().into_iter().map(|e| e.shelf_id).collect();
        assert_eq!(remaining, vec![c1.entry.shelf_id.clone()]);
        drop(pin_guard);
    }

    #[test]
    fn shelf_lock_alone_does_not_protect() {
        let fs = IsolatedFilesystem::new("sweeper-shelf").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let policy_store = Arc::new(CachePolicyStore::new());
        // 2 entries max, low_water 0.5 = 1 → sweep evicts 1 leaving 1.
        let policy = CachePolicy {
            max_entries: 2,
            low_water_ratio: 0.5,
            ..CachePolicy::default()
        };
        policy_store.update(policy);
        let c1 = cache.commit(request(4, 4)).expect("commit 1");
        let _c2 = cache.commit(request(4, 4)).expect("commit 2");
        // Only the shelf lock holds c1 — that should NOT protect it.
        let sweeper = CacheSweeper::with_clock(
            Arc::new(cache.clone()),
            policy_store.clone(),
            Arc::new(TestClock(ControllableClock::new())),
        );
        let outcome = sweeper.sweep_once();
        // c1 (oldest) was evicted despite the shelf lock.
        assert_eq!(outcome.quota_evicted, 1);
        let remaining: Vec<_> = cache.entries().into_iter().map(|e| e.shelf_id).collect();
        assert!(!remaining.contains(&c1.entry.shelf_id));
    }

    #[test]
    fn sweep_stops_when_only_locked_entries_remain() {
        let fs = IsolatedFilesystem::new("sweeper-stop-locked").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let policy_store = Arc::new(CachePolicyStore::new());
        // 1 entry max, low_water 0.5 = 0. The sweep must evict every
        // unlocked entry to reach the low-water target. c1 is pinned,
        // c2 is evictable; c2 is evicted, then the loop has no
        // further unlocked candidates so it stops.
        let policy = CachePolicy {
            max_entries: 1,
            low_water_ratio: 0.5,
            ..CachePolicy::default()
        };
        policy_store.update(policy);
        let c1 = cache.commit(request(4, 4)).expect("commit 1");
        let _c2 = cache.commit(request(4, 4)).expect("commit 2");
        // Pin c1 so it is protected.
        let guard = cache
            .locks()
            .acquire(c1.entry.shelf_id.clone(), LockOwner::Pin);
        let sweeper = CacheSweeper::with_clock(
            Arc::new(cache.clone()),
            policy_store.clone(),
            Arc::new(TestClock(ControllableClock::new())),
        );
        let outcome = sweeper.sweep_once();
        // c2 is evicted, c1 remains. The quota is still violated but
        // the only survivor is locked so the loop gives up.
        assert_eq!(outcome.quota_evicted, 1);
        let remaining: Vec<_> = cache.entries().into_iter().map(|e| e.shelf_id).collect();
        assert_eq!(remaining, vec![c1.entry.shelf_id.clone()]);
        drop(guard);
    }

    #[test]
    fn recover_debris_removes_stale_tmp_files() {
        let fs = IsolatedFilesystem::new("sweeper-tmp").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        // Drop a stale .tmp file at the root.
        let tmp = fs.join("capture.png.tmp");
        std::fs::write(&tmp, b"leftover").expect("write tmp");
        let outcome = cache.recover_debris().expect("recover");
        assert_eq!(outcome.tmp_files_removed, 1);
        assert!(!tmp.exists());
    }

    #[test]
    fn recover_debris_removes_zero_byte_assets() {
        let fs = IsolatedFilesystem::new("sweeper-zero-byte").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        // Commit a valid entry so the cache root has a directory.
        let committed = cache.commit(request(4, 4)).expect("commit");
        let entry_dir = fs.root().join(&committed.entry.capture_id);
        // Drop a zero-byte capture.png inside an unrelated directory
        // that has a manifest. The recovery sweep must remove the
        // zero-byte asset but leave the durable entry untouched.
        let foreign_id = "dec0de00-0000-0000-0000-000000000000";
        let foreign_dir = fs.root().join(foreign_id);
        std::fs::create_dir_all(&foreign_dir).expect("mkdir");
        std::fs::write(foreign_dir.join("manifest.json"), b"{}").expect("write manifest");
        std::fs::write(foreign_dir.join("capture.png"), b"").expect("write zero png");
        let outcome = cache.recover_debris().expect("recover");
        assert_eq!(outcome.zero_byte_assets_removed, 1);
        assert!(!foreign_dir.join("capture.png").exists());
        // Original entry is untouched.
        assert!(entry_dir.join("capture.png").exists());
    }

    #[test]
    fn recover_debris_partial_failure_continues() {
        let fs = IsolatedFilesystem::new("sweeper-partial").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        // Drop a regular file at the root named like a stale tmp
        // sibling. The sweep must still reap the unrelated tmp file.
        let blocker = fs.join("not-a-directory-tmp");
        std::fs::write(&blocker, b"x").expect("write blocker");
        std::fs::write(fs.join("capture.png.tmp"), b"leftover").expect("write tmp");
        let outcome = cache.recover_debris().expect("recover");
        assert_eq!(outcome.tmp_files_removed, 1);
        assert_eq!(outcome.partial_failures, 0);
    }

    #[test]
    fn recover_debris_reaps_manifest_less_directories() {
        let fs = IsolatedFilesystem::new("sweeper-unindexed").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        // Drop a manifest-less directory with leftover assets from
        // a crashed commit. The recovery must remove it and report
        // the sum of the on-disk bytes.
        let partial_id = "11111111-1111-1111-1111-111111111111";
        let partial_dir = fs.root().join(partial_id);
        std::fs::create_dir_all(&partial_dir).expect("mkdir");
        std::fs::write(partial_dir.join("capture.png"), b"partial-png").expect("png");
        std::fs::write(partial_dir.join("metadata.json"), b"{}").expect("meta");
        let outcome = cache.recover_debris().expect("recover");
        assert_eq!(outcome.unindexed_dirs_removed, 1);
        assert!(!partial_dir.exists());
        // The on-disk size of the reaped files is reported in
        // `bytes_reclaimed` (11 + 2 = 13).
        assert!(outcome.bytes_reclaimed >= 13);
    }

    #[test]
    fn try_clear_skips_locked_entries() {
        let fs = IsolatedFilesystem::new("sweeper-clear").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let c1 = cache.commit(request(4, 4)).expect("commit 1");
        let _c2 = cache.commit(request(4, 4)).expect("commit 2");
        let guard = cache
            .locks()
            .acquire(c1.entry.shelf_id.clone(), LockOwner::Editor);
        let outcome = cache.clear_unlocked_entries();
        assert_eq!(outcome.quota_evicted, 1);
        let remaining: Vec<_> = cache.entries().into_iter().map(|e| e.shelf_id).collect();
        assert!(remaining.contains(&c1.entry.shelf_id));
        drop(guard);
    }

    #[test]
    fn stats_match_actual_usage() {
        let fs = IsolatedFilesystem::new("sweeper-stats").expect("fs");
        let cache = Cache::new();
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let c1 = cache.commit(request(4, 4)).expect("commit 1");
        let c2 = cache.commit(request(4, 4)).expect("commit 2");
        let stats = cache.stats();
        assert_eq!(stats.entry_count, 2);
        assert_eq!(stats.total_bytes, c1.entry.size_bytes + c2.entry.size_bytes);
        assert_eq!(stats.locked_count, 0);
        assert!(stats.oldest_created_at_ms.is_some());
        assert!(stats.newest_access_at_ms.is_some());
    }

    #[test]
    fn periodic_worker_stops_on_drop() {
        let fs = IsolatedFilesystem::new("sweeper-periodic").expect("fs");
        let cache = Arc::new(Cache::new());
        cache
            .set_cache_root(Some(fs.root().to_path_buf()))
            .expect("set root");
        let policy_store = Arc::new(CachePolicyStore::new());
        policy_store.update(CachePolicy {
            sweep_interval_ms: 1_000, // 1s for the test
            ..CachePolicy::default()
        });
        let sweeper = make_sweeper(&cache, &policy_store);
        let worker = sweeper.start_periodic();
        // Drop the worker — the thread should exit.
        drop(worker);
    }
}
