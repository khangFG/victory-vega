//! Host-memory pressure tracking and `E_OUTOFMEMORY` backoff (harvested from
//! `VIctory-RDNA1-Hybrid` and tuned for low-VRAM Vega APUs / 16 GB systems).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use windows::Win32::Graphics::Dxgi::{IDXGIAdapter1, IDXGIAdapter3};
use windows::core::Interface;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

const GIB: u64 = 1 << 30;
const LOW_VRAM_CEILING: u64 = 2 * GIB;

static PRESSURED: AtomicBool = AtomicBool::new(false);
static LAST_SAMPLE_MS: AtomicU64 = AtomicU64::new(0);
static RETRY_UPLOAD_AT: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn memory_status() -> Option<MEMORYSTATUSEX> {
    // SAFETY: initializes a correctly sized Windows status structure.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: `status` is initialized with `dwLength` set to its byte size.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    (ok != 0).then_some(status)
}

/// Records an `E_OUTOFMEMORY` allocation failure and backs off speculative
/// staging uploads for 500 ms so the runtime can drain or evict.
pub(crate) fn allocation_error(error: &windows::core::Error) {
    if error.code() == windows::Win32::Foundation::E_OUTOFMEMORY {
        RETRY_UPLOAD_AT.fetch_max(now_ms().saturating_add(500), Ordering::Relaxed);
    }
}

/// Whether speculative uploads should pause briefly after a recent `E_OUTOFMEMORY`.
pub(crate) fn upload_refused() -> bool {
    now_ms() < RETRY_UPLOAD_AT.load(Ordering::Relaxed)
}

fn next_pressure(was: bool, physical: u64, commit: u64) -> bool {
    if was {
        physical < 5 * GIB || commit < 6 * GIB
    } else {
        physical <= 3 * GIB || commit <= 4 * GIB
    }
}

/// Samples Windows physical/commit headroom at most once per second and updates
/// the pressure state.
pub(crate) fn refresh() {
    let now = now_ms();
    let prev = LAST_SAMPLE_MS.load(Ordering::Relaxed);
    if now.saturating_sub(prev) < 1000
        || LAST_SAMPLE_MS
            .compare_exchange(prev, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    let Some(status) = memory_status() else {
        return;
    };
    let (physical, commit) = (status.ullAvailPhys, status.ullAvailPageFile);
    let was = pressured();
    let pressure = next_pressure(was, physical, commit);
    if pressure != was {
        PRESSURED.store(pressure, Ordering::Relaxed);
        if pressure {
            tracing::warn!(
                physical_free_mib = physical >> 20,
                available_commit_mib = commit >> 20,
                "host memory pressure detected"
            );
        } else {
            tracing::info!(
                physical_free_mib = physical >> 20,
                available_commit_mib = commit >> 20,
                "host memory pressure cleared"
            );
        }
    }
}

pub(crate) fn pressured() -> bool {
    PRESSURED.load(Ordering::Relaxed)
}

/// Returns `true` when the adapter reports 2 GiB or less of dedicated VRAM
/// (typical for integrated Vega APUs reporting 512 MiB - 2 GiB UMA).
pub(crate) fn low_vram_adapter(adapter: Option<&IDXGIAdapter3>) -> bool {
    let Some(adapter) = adapter else {
        return false;
    };
    let Ok(a1) = adapter.cast::<IDXGIAdapter1>() else {
        return false;
    };
    // SAFETY: plain DXGI adapter descriptor query.
    let Ok(desc) = (unsafe { a1.GetDesc1() }) else {
        return false;
    };
    desc.DedicatedVideoMemory > 0 && desc.DedicatedVideoMemory as u64 <= LOW_VRAM_CEILING
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn either_constraint_enters_pressure_and_both_must_recover() {
        assert!(next_pressure(false, 12 * GIB, 4 * GIB));
        assert!(next_pressure(false, 3 * GIB, 16 * GIB));
        assert!(next_pressure(true, 4 * GIB, 7 * GIB));
        assert!(next_pressure(true, 6 * GIB, 5 * GIB));
        assert!(!next_pressure(true, 5 * GIB, 6 * GIB));
        assert!(!next_pressure(false, 4 * GIB, 5 * GIB));
    }

    #[test]
    fn oom_backoff_trips_on_e_outofmemory() {
        let other = windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL);
        allocation_error(&other);
        let oom = windows::core::Error::from_hresult(windows::Win32::Foundation::E_OUTOFMEMORY);
        allocation_error(&oom);
        assert!(upload_refused());
        // Reset backoff so subsequent tests are unaffected.
        RETRY_UPLOAD_AT.store(0, Ordering::Relaxed);
    }
}
