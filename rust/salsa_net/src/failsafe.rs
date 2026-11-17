#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub const FS_PA_BAD_PKT: u32 = 1;
pub const FS_PW_BAD_PKT: u32 = 2;
pub const FS_FD_QUOTA: u32 = 3;
pub const FS_BT_FALLBACK: u32 = 4;
pub const FS_MANUAL_RESET: u32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FailsafeStats {
    pub is_rust: bool,
    pub armed: bool,
    pub trips_total: u64,
    pub pa_quarantined: u64,
    pub pw_quarantined: u64,
    pub bt_fallbacks: u64,
    pub fd_leaks_blocked: u64,
    pub manual_resets: u64,
    pub last_reason: [u8; 96],
}

static TRIPS_TOTAL: AtomicU64 = AtomicU64::new(0);
static PA_QUARANTINED: AtomicU64 = AtomicU64::new(0);
static PW_QUARANTINED: AtomicU64 = AtomicU64::new(0);
static BT_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static FD_LEAKS_BLOCKED: AtomicU64 = AtomicU64::new(0);
static MANUAL_RESETS: AtomicU64 = AtomicU64::new(0);
static LAST_REASON: Mutex<[u8; 96]> = Mutex::new([0u8; 96]);

pub fn record_trip(kind: u32, reason: &[u8]) {
    TRIPS_TOTAL.fetch_add(1, Ordering::Relaxed);
    match kind {
        FS_PA_BAD_PKT => {
            PA_QUARANTINED.fetch_add(1, Ordering::Relaxed);
        }
        FS_PW_BAD_PKT => {
            PW_QUARANTINED.fetch_add(1, Ordering::Relaxed);
        }
        FS_FD_QUOTA => {
            FD_LEAKS_BLOCKED.fetch_add(1, Ordering::Relaxed);
        }
        FS_BT_FALLBACK => {
            BT_FALLBACKS.fetch_add(1, Ordering::Relaxed);
        }
        FS_MANUAL_RESET => {
            MANUAL_RESETS.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
    if !reason.is_empty() {
        if let Ok(mut guard) = LAST_REASON.lock() {
            guard.fill(0);
            let end = reason.iter().position(|&b| b == 0).unwrap_or(reason.len());
            let n = end.min(95);
            guard[..n].copy_from_slice(&reason[..n]);
        }
    }
}

pub fn get_stats() -> FailsafeStats {
    let reason_copy = LAST_REASON.lock().map(|g| *g).unwrap_or([0u8; 96]);
    FailsafeStats {
        is_rust: true,
        armed: true,
        trips_total: TRIPS_TOTAL.load(Ordering::Relaxed),
        pa_quarantined: PA_QUARANTINED.load(Ordering::Relaxed),
        pw_quarantined: PW_QUARANTINED.load(Ordering::Relaxed),
        bt_fallbacks: BT_FALLBACKS.load(Ordering::Relaxed),
        fd_leaks_blocked: FD_LEAKS_BLOCKED.load(Ordering::Relaxed),
        manual_resets: MANUAL_RESETS.load(Ordering::Relaxed),
        last_reason: reason_copy,
    }
}

pub fn reset_stats() {
    TRIPS_TOTAL.store(0, Ordering::Relaxed);
    PA_QUARANTINED.store(0, Ordering::Relaxed);
    PW_QUARANTINED.store(0, Ordering::Relaxed);
    BT_FALLBACKS.store(0, Ordering::Relaxed);
    FD_LEAKS_BLOCKED.store(0, Ordering::Relaxed);
    MANUAL_RESETS.store(0, Ordering::Relaxed);
    if let Ok(mut guard) = LAST_REASON.lock() {
        guard.fill(0);
    }
}
