//! Renderer port #5: raw constant buffer reads (`VICTORY_PORT_CBV=1`, off by default).
//!
//! Table and resource-heap constant buffers already read via raw `R32_TYPELESS`
//! SRVs in `victory-gpu` (which read in place when `VICTORY_PORT_CONSTANTS=1`).
//! Enabling `VICTORY_PORT_CBV=1` also lowers root constant buffers
//! (`RootType::Descriptor` CBVs) to direct raw root SRVs (`t<slot>` in
//! `ROOT_CBV_SPACE`), which only need 4-byte alignment so unaligned root CBVs
//! bind directly without 64 KiB `scratch` + `defer_fill` upload copies.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Whether renderer port #5 is on (`VICTORY_PORT_CBV=1`).
pub(crate) fn on() -> bool {
    victory_shader::emit::constants_raw()
}

/// Whether a constant buffer at host address `gpu` satisfies raw SRV alignment.
pub(crate) fn raw_ok(gpu: u64) -> bool {
    gpu.is_multiple_of(16)
}

/// Since the last frame line:
/// - `[0..2]`: root constant buffers bound directly in place (count, bytes)
/// - `[2..4]`: root constant buffers bound directly from a host copy (count, bytes)
/// - `[4..6]`: root constant buffers copied into upload scratch (count, bytes)
/// - `[6..8]`: table constant buffer views created in place (count, bytes)
/// - `[8..10]`: table constant buffer views created over host copies (count, bytes)
static STATS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];

/// Records a direct root constant buffer bind (`Set*RootShaderResourceView` /
/// `Set*RootConstantBufferView`).
pub(crate) fn root_direct(in_place: bool, len: u64) {
    let base = if in_place { 0 } else { 2 };
    STATS[base].fetch_add(1, Relaxed);
    STATS[base + 1].fetch_add(len, Relaxed);
}

/// Records a root constant buffer copied via `list.scratch` + `defer_fill`.
pub(crate) fn root_copied(len: u64) {
    STATS[4].fetch_add(1, Relaxed);
    STATS[5].fetch_add(len, Relaxed);
}

/// Records a table `View::Constant` raw SRV descriptor created over an in-place
/// or copied shadow buffer.
pub(crate) fn table_view(in_place: bool, size: u32) {
    let base = if in_place { 6 } else { 8 };
    STATS[base].fetch_add(1, Relaxed);
    STATS[base + 1].fetch_add(u64::from(size), Relaxed);
}

/// Logs per-frame `VICTORY_PORT_CBV` counters on the periodic FPS report line.
pub(crate) fn log(frames: f64) {
    if !on() {
        return;
    }
    let scale = frames.max(1e-9);
    let s: Vec<f64> = STATS
        .iter()
        .map(|a| a.swap(0, Relaxed) as f64 / scale)
        .collect();
    tracing::info!(
        "renderer port 5 (VICTORY_PORT_CBV=1) per frame: root CBVs direct in place {:.0} ({:.1} KB), \
         direct host copy {:.0} ({:.1} KB), scratch copied {:.0} ({:.1} KB); \
         table constant SRVs in place {:.0} ({:.1} KB), host copy {:.0} ({:.1} KB)",
        s[0],
        s[1] / 1024.0,
        s[2],
        s[3] / 1024.0,
        s[4],
        s[5] / 1024.0,
        s[6],
        s[7] / 1024.0,
        s[8],
        s[9] / 1024.0,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_views_need_sixteen_bytes() {
        assert!(raw_ok(0x3500_0010));
        assert!(raw_ok(0x3500_0100));
        assert!(!raw_ok(0x3500_0008));
        assert!(!raw_ok(0x3500_0004));
    }
}
