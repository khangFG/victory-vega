//! Compute + vertex emulation resources for GPUs without hardware mesh
//! shaders (and when `VICTORY_MESH_EMULATION=1` is set).
//!
//! An emulated mesh pipeline runs in two host steps on a shared VRAM buffer:
//! 1. The translated mesh compute shader (`cs_6_6`) writes packed primitive
//!    indices, vertex attributes, and per-primitive attributes per group into
//!    `output` (`u0, space 3`).
//! 2. The generated unpack vertex shader (`vs_6_6`) reads `output` (`t0, space
//!    3`) by `SV_VertexID` and feeds the title's original pixel shader.
//!
//! Indirect mesh dispatches (`ExecuteIndirect`) additionally run a tiny
//! compute preparation pass (`PREP_SOURCE`) to sum group counts across
//! indirect records, build `slot_map` (`t0, space 3`) when `max_records > 1`,
//! and emit both host `Dispatch` and `DrawInstanced` indirect arguments into
//! `indirect_args`.

use std::sync::OnceLock;

use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::Win32::Graphics::Direct3D12::*;
use windows::core::s;

use crate::host::{Host, HostList, host, name, transition};
use crate::indirect::uav_barrier;

const SLOT_MAP_BYTES: u64 = 65_536 * 4;
const INDIRECT_ARGS_BYTES: u64 = 256;
const OUTPUT_FALLBACK_SIZES: &[u64] = &[128 << 20, 64 << 20, 32 << 20, 16 << 20, 8 << 20];
const LOW_VRAM_OUTPUT_FALLBACK_SIZES: &[u64] = &[16 << 20, 8 << 20, 4 << 20];

const PREP_SOURCE: &str = r#"
#define RS "RootConstants(num32BitConstants=6, b0), UAV(u0), UAV(u1), UAV(u2), UAV(u3)"
cbuffer Info : register(b0) {
    uint stride;
    uint max_records;
    uint max_primitives;
    uint group_cap;
    uint has_count;
    uint build_map;
};
RWByteAddressBuffer args : register(u0);
RWByteAddressBuffer count_buf : register(u1);
RWByteAddressBuffer out_args : register(u2);
RWByteAddressBuffer slot_map : register(u3);
[RootSignature(RS)]
[numthreads(1, 1, 1)]
void main() {
    uint n = has_count != 0 ? min(count_buf.Load(0), max_records) : max_records;
    uint cap = min(group_cap, 65535u);
    uint total = 0;
    for (uint r = 0; r < n; ++r) {
        if (total >= cap) {
            break;
        }
        uint3 g = args.Load3(r * stride);
        uint gx = (g.y != 0 && g.z != 0) ? g.x : 0;
        uint left = cap - total;
        uint take = min(gx, left);
        if (build_map != 0) {
            for (uint k = 0; k < take; ++k) {
                slot_map.Store((total + k) * 4, k);
            }
        }
        total += take;
    }
    out_args.Store4(0, uint4(total, 1, 1, 0));
    out_args.Store4(16, uint4(total * max_primitives * 3, 1, 0, 0));
}
"#;

pub(crate) struct Buffers {
    pub(crate) output: ID3D12Resource,
    pub(crate) output_gpu: u64,
    pub(crate) output_bytes: u64,
    pub(crate) slot_map: ID3D12Resource,
    pub(crate) slot_map_gpu: u64,
    pub(crate) indirect_args: ID3D12Resource,
    pub(crate) indirect_args_gpu: u64,
    pub(crate) dummy_srv: ID3D12Resource,
    pub(crate) dummy_srv_gpu: u64,
    prep_root: ID3D12RootSignature,
    prep_pso: ID3D12PipelineState,
}

// SAFETY: D3D12 resources, root signatures, and pipeline states are free-threaded.
unsafe impl Send for Buffers {}
// SAFETY: as above.
unsafe impl Sync for Buffers {}

pub(crate) fn buffers() -> Option<&'static Buffers> {
    static BUFFERS: OnceLock<Option<Buffers>> = OnceLock::new();
    BUFFERS
        .get_or_init(|| match make() {
            Some(b) => Some(b),
            None => {
                tracing::warn!("no mesh emulation buffers; emulated mesh dispatches dropped");
                None
            }
        })
        .as_ref()
}

fn configured_output_bytes() -> Option<u64> {
    let raw = std::env::var("VICTORY_MESH_BUFFER_MB").ok()?;
    let mb = raw.trim().parse::<u64>().ok()?;
    if (1..=1024).contains(&mb) {
        Some(mb << 20)
    } else {
        tracing::warn!(
            value = %raw,
            "VICTORY_MESH_BUFFER_MB out of range (1..=1024); using default"
        );
        None
    }
}

fn allocate_output(host: &Host) -> Option<(ID3D12Resource, u64)> {
    if let Some(bytes) = configured_output_bytes()
        && let Some(buf) = host.unordered_buffer(bytes, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS)
    {
        return Some((buf, bytes));
    }
    let sizes = if host.low_vram() {
        LOW_VRAM_OUTPUT_FALLBACK_SIZES
    } else {
        OUTPUT_FALLBACK_SIZES
    };
    for &bytes in sizes {
        if let Some(buf) = host.unordered_buffer(bytes, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS)
        {
            return Some((buf, bytes));
        }
    }
    None
}

fn compile_prep(
    device: &ID3D12Device,
) -> windows::core::Result<(ID3D12RootSignature, ID3D12PipelineState)> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: static HLSL source and entry strings live through the call; out pointers are ours.
    let compiled = unsafe {
        D3DCompile(
            PREP_SOURCE.as_ptr().cast(),
            PREP_SOURCE.len(),
            s!("mesh_indirect_prep"),
            None,
            None,
            s!("main"),
            s!("cs_5_1"),
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(e) = compiled {
        if let Some(errors) = errors {
            // SAFETY: the error blob owns its bytes.
            let text = unsafe {
                std::slice::from_raw_parts(
                    errors.GetBufferPointer().cast::<u8>(),
                    errors.GetBufferSize(),
                )
            };
            tracing::warn!("mesh_indirect_prep: {}", String::from_utf8_lossy(text));
        }
        return Err(e);
    }
    let code = code.ok_or_else(windows::core::Error::empty)?;
    // SAFETY: `code` holds valid compiled DXBC with an embedded root signature.
    unsafe {
        let bytes =
            std::slice::from_raw_parts(code.GetBufferPointer().cast::<u8>(), code.GetBufferSize());
        let root: ID3D12RootSignature = device.CreateRootSignature(0, bytes)?;
        let pso = device.CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
            pRootSignature: std::mem::transmute_copy(&root),
            CS: D3D12_SHADER_BYTECODE {
                pShaderBytecode: bytes.as_ptr().cast(),
                BytecodeLength: bytes.len(),
            },
            ..Default::default()
        })?;
        Ok((root, pso))
    }
}

fn make() -> Option<Buffers> {
    let host = host()?;
    let (output, output_bytes) = allocate_output(host)?;
    let slot_map =
        host.unordered_buffer(SLOT_MAP_BYTES, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS)?;
    let indirect_args = host.unordered_buffer(
        INDIRECT_ARGS_BYTES,
        D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
    )?;
    let (dummy_srv, dummy_ptr) = host.upload_buffer(256)?;
    // SAFETY: `dummy_ptr` points to 256 zeroed bytes of a live upload buffer.
    unsafe { std::ptr::write_bytes(dummy_ptr, 0, 256) };
    let (prep_root, prep_pso) = compile_prep(&host.device)
        .inspect_err(|e| tracing::warn!(error = %e, "mesh_indirect_prep pipeline failed"))
        .ok()?;
    name(&output, || format!("mesh emu output {output_bytes:#x}"));
    name(&slot_map, || "mesh emu slot_map".into());
    name(&indirect_args, || "mesh emu indirect_args".into());
    name(&dummy_srv, || "mesh emu dummy_srv".into());
    // SAFETY: `output`, `slot_map`, `indirect_args`, and `dummy_srv` are live buffer resources.
    let (output_gpu, slot_map_gpu, indirect_args_gpu, dummy_srv_gpu) = unsafe {
        (
            output.GetGPUVirtualAddress(),
            slot_map.GetGPUVirtualAddress(),
            indirect_args.GetGPUVirtualAddress(),
            dummy_srv.GetGPUVirtualAddress(),
        )
    };
    tracing::info!(
        output_mb = output_bytes >> 20,
        low_vram = host.low_vram(),
        "mesh shader emulation buffers ready"
    );
    Some(Buffers {
        output,
        output_gpu,
        output_bytes,
        slot_map,
        slot_map_gpu,
        indirect_args,
        indirect_args_gpu,
        dummy_srv,
        dummy_srv_gpu,
        prep_root,
        prep_pso,
    })
}

impl Buffers {
    /// Maximum number of mesh thread groups that fit in `output` for one chunk.
    pub(crate) fn group_cap(&self, group_bytes: u32) -> u32 {
        (self.output_bytes / u64::from(group_bytes.max(4))).min(65_535) as u32
    }

    /// Computes total indirect group count (clamped to `group_cap`), populates
    /// `slot_map` when `max_records > 1`, and writes host indirect dispatch and
    /// draw arguments into `indirect_args`. Leaves `indirect_args` in
    /// `INDIRECT_ARGUMENT` and (if `max_records > 1`) `slot_map` in
    /// `NON_PIXEL_SHADER_RESOURCE`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_indirect(
        &self,
        list: &HostList,
        args_gpu: u64,
        count_gpu: Option<u64>,
        stride: u32,
        max_records: u32,
        max_primitives: u32,
        group_cap: u32,
    ) {
        let l = &list.list;
        let build_map = max_records > 1;
        let constants = [
            stride,
            max_records,
            max_primitives,
            group_cap,
            u32::from(count_gpu.is_some()),
            u32::from(build_map),
        ];
        // SAFETY: `list` is open; `args_gpu`, `count_gpu`, `indirect_args_gpu`,
        // and `slot_map_gpu` point to live buffer allocations kept for the submission.
        unsafe {
            l.ResourceBarrier(&[uav_barrier()]);
            l.SetComputeRootSignature(&self.prep_root);
            l.SetPipelineState(&self.prep_pso);
            l.SetComputeRoot32BitConstants(0, 6, constants.as_ptr().cast(), 0);
            l.SetComputeRootUnorderedAccessView(1, args_gpu);
            l.SetComputeRootUnorderedAccessView(2, count_gpu.unwrap_or(args_gpu));
            l.SetComputeRootUnorderedAccessView(3, self.indirect_args_gpu);
            l.SetComputeRootUnorderedAccessView(4, self.slot_map_gpu);
            l.Dispatch(1, 1, 1);
            let args_barrier = transition(
                &self.indirect_args,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_INDIRECT_ARGUMENT,
            );
            if build_map {
                let map_barrier = transition(
                    &self.slot_map,
                    D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                    D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                );
                l.ResourceBarrier(&[args_barrier, map_barrier]);
            } else {
                l.ResourceBarrier(&[args_barrier]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prep_shader_builds_and_runs_indirect() {
        let Some(host) = host() else {
            return; // no host GPU here
        };
        let b = buffers().expect("mesh emulation buffers");
        let (args_buf, args_ptr) = host.upload_buffer(256).expect("upload args");
        // SAFETY: `args_ptr` covers 256 bytes; write 3 indirect mesh records
        // (group counts 2, 0, 3) and a count of 2 at offset 128 (same buffer!).
        unsafe {
            let words = args_ptr.cast::<u32>();
            // Record 0: 2 groups, y=1, z=1
            words.add(0).write(2);
            words.add(1).write(1);
            words.add(2).write(1);
            // Record 1: 0 groups, y=1, z=1
            words.add(3).write(0);
            words.add(4).write(1);
            words.add(5).write(1);
            // Record 2: 3 groups, y=1, z=1
            words.add(6).write(3);
            words.add(7).write(1);
            words.add(8).write(1);
            // Count at byte 128 (dword 32): 3 records
            words.add(32).write(3);
        }
        // `args_buf` is an UPLOAD buffer, which cannot be bound as a UAV in D3D12;
        // copy into a DEFAULT buffer with ALLOW_UNORDERED_ACCESS just as Shadow uses shared_buffer.
        let (args_shared, args_shared_ptr) = host.shared_buffer(256).expect("shared args");
        // SAFETY: copy 256 bytes into mapped shared buffer.
        unsafe { std::ptr::copy_nonoverlapping(args_ptr, args_shared_ptr, 256) };
        drop(args_buf);
        // SAFETY: live resource.
        let args_gpu = unsafe { args_shared.GetGPUVirtualAddress() };

        let (readback_args, rb_args_ptr) = host.shared_buffer(256).expect("readback args");
        let (readback_map, rb_map_ptr) = host.shared_buffer(256).expect("readback map");

        // Case 1: max_records = 3, with count = 3 (same resource!), group_cap = 4 (clamps 2+0+3=5 down to 4 -> 2 from rec 0, 2 from rec 2).
        let list = host.new_list().expect("list");
        b.prepare_indirect(&list, args_gpu, Some(args_gpu + 128), 12, 3, 10, 4);
        // SAFETY: open list; transitions back to COPY_SOURCE for readback.
        unsafe {
            list.list.ResourceBarrier(&[
                transition(
                    &b.indirect_args,
                    D3D12_RESOURCE_STATE_INDIRECT_ARGUMENT,
                    D3D12_RESOURCE_STATE_COPY_SOURCE,
                ),
                transition(
                    &b.slot_map,
                    D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                    D3D12_RESOURCE_STATE_COPY_SOURCE,
                ),
            ]);
            list.list
                .CopyBufferRegion(&readback_args, 0, &b.indirect_args, 0, 32);
            list.list
                .CopyBufferRegion(&readback_map, 0, &b.slot_map, 0, 32);
            list.list.ResourceBarrier(&[
                transition(
                    &b.indirect_args,
                    D3D12_RESOURCE_STATE_COPY_SOURCE,
                    D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                ),
                transition(
                    &b.slot_map,
                    D3D12_RESOURCE_STATE_COPY_SOURCE,
                    D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                ),
            ]);
        }
        list.close();
        host.execute(&[&list]);
        // SAFETY: GPU execution finished; read mapped buffers.
        let (out_words, map_words) = unsafe {
            (
                std::slice::from_raw_parts(rb_args_ptr.cast::<u32>(), 8).to_vec(),
                std::slice::from_raw_parts(rb_map_ptr.cast::<u32>(), 4).to_vec(),
            )
        };
        assert_eq!(&out_words[0..3], &[4, 1, 1]);
        assert_eq!(&out_words[4..8], &[4 * 10 * 3, 1, 0, 0]);
        assert_eq!(map_words, vec![0, 1, 0, 1]);

        // Case 2: max_records = 1, no count buffer, group_cap = 10 -> 2 groups.
        list.reset();
        b.prepare_indirect(&list, args_gpu, None, 12, 1, 8, 10);
        // SAFETY: open list; transition indirect_args only (slot_map untouched for max == 1).
        unsafe {
            list.list.ResourceBarrier(&[transition(
                &b.indirect_args,
                D3D12_RESOURCE_STATE_INDIRECT_ARGUMENT,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            )]);
            list.list
                .CopyBufferRegion(&readback_args, 0, &b.indirect_args, 0, 32);
            list.list.ResourceBarrier(&[transition(
                &b.indirect_args,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            )]);
        }
        list.close();
        host.execute(&[&list]);
        // SAFETY: GPU execution finished.
        let out_words =
            unsafe { std::slice::from_raw_parts(rb_args_ptr.cast::<u32>(), 8).to_vec() };
        assert_eq!(&out_words[0..3], &[2, 1, 1]);
        assert_eq!(&out_words[4..8], &[2 * 8 * 3, 1, 0, 0]);
    }
}
