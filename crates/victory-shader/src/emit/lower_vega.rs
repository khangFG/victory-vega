use crate::emit::wave;
use crate::native::UsedResource;
use super::*;

impl<'a> Lower<'a> {
    pub(super) fn mesh_emu_init(&mut self) -> Result<(), Why> {
        let (threads, raw_verts, raw_prims, topology, vertex_dwords, prim_dwords) = {
            let m = self.mesh_emu.as_ref().expect("mesh emu");
            (
                m.threads,
                m.max_vertices,
                m.max_primitives,
                m.topology,
                m.vertex_dwords,
                m.prim_dwords,
            )
        };
        if topology != 2 {
            return Err("a mesh shader not drawing triangles");
        }
        if threads == 0 || threads > 1024 || raw_verts > 256 || raw_prims > 256 {
            return Err("a mesh shader beyond the host's limits");
        }
        let max_vertices = raw_verts.min(threads);
        let max_primitives = raw_prims.min(threads);
        if max_vertices == 0 || max_primitives == 0 {
            return Err("an empty mesh shader output");
        }
        let index_bytes = max_primitives * 4;
        let vertex_stride_bytes = vertex_dwords * 4;
        let prim_offset_bytes = index_bytes + max_vertices * vertex_stride_bytes;
        let prim_stride_bytes = prim_dwords * 4;
        let group_bytes = prim_offset_bytes + max_primitives * prim_stride_bytes;

        let e = self.s.entry;
        let i32t = self.s.t.i32;
        let uav = self.s.emu_uav_handle();
        let k0 = self.s.m.i32(0);
        let local_group = self.s.op(e, "groupId.i32", Attr::ReadNone, i32t, 94, &[k0]);
        let flat = self.mesh_thread();
        let group_stride = self.s.m.i32(group_bytes);
        let group_base = self.s.m.bin(e, Bin::Mul, local_group, group_stride);
        let flat_lt_verts = (threads > max_vertices).then(|| {
            let mv = self.s.m.i32(max_vertices);
            self.s.m.cmp(e, pred::IULT, flat, mv)
        });
        let flat_lt_prims = (threads > max_primitives).then(|| {
            let mp = self.s.m.i32(max_primitives);
            self.s.m.cmp(e, pred::IULT, flat, mp)
        });
        let m = self.mesh_emu.as_mut().expect("mesh emu");
        m.max_vertices = max_vertices;
        m.max_primitives = max_primitives;
        m.vals = Some(MeshEmuVals {
            uav,
            local_group,
            group_base,
            flat_lt_verts,
            flat_lt_prims,
            index_bytes,
            vertex_stride_bytes,
            prim_offset_bytes,
            prim_stride_bytes,
        });
        Ok(())
    }

    /// Initializes every primitive index word in the group slice as culled
    /// (`bit 31 = 1`) so unexported slots never draw stale data.
    pub(super) fn mesh_emu_prologue(&mut self) {
        let (uav, group_base, flat_lt_prims) = {
            let v = self
                .mesh_emu
                .as_ref()
                .expect("mesh emu")
                .vals
                .as_ref()
                .expect("mesh emu init");
            (v.uav, v.group_base, v.flat_lt_prims)
        };
        let flat = self.mesh_thread();
        let at = self.begin_guarded(flat_lt_prims);
        let c = self.cur;
        let k2 = self.s.m.i32(2);
        let byte_off = self.s.m.bin(c, Bin::Shl, flat, k2);
        let addr = self.s.m.bin(c, Bin::Add, group_base, byte_off);
        let undef = self.s.m.undef(self.s.t.i32);
        let culled = self.s.m.i32(0x8000_0000);
        let mask1 = self.s.m.i8(1);
        let align4 = self.s.m.i32(4);
        let void = self.s.t.void;
        self.s.op(
            c,
            "rawBufferStore.i32",
            Attr::NoUnwind,
            void,
            140,
            &[uav, addr, undef, culled, undef, undef, undef, mask1, align4],
        );
        self.end_guarded(at, &[]);
    }

    pub(super) fn combine_guards(&mut self, a: Option<V>, b: Option<V>) -> Option<V> {
        match (a, b) {
            (Some(x), Some(y)) => Some(self.s.m.bin(self.cur, Bin::And, x, y)),
            (Some(x), None) | (None, Some(x)) => Some(x),
            (None, None) => None,
        }
    }

    /// Mesh: the thread's index in the group, which is its vertex and
    /// primitive index.
    pub(super) fn mesh_thread(&mut self) -> V {
        if let Some(f) = self.flat {
            return f;
        }
        let e = self.s.entry;
        let f = self.s.op(
            e,
            "flattenedThreadIdInGroup.i32",
            Attr::ReadNone,
            self.s.t.i32,
            96,
            &[],
        );
        self.flat = Some(f);
        f
    }

    /// The console wave size.
    pub(super) fn wave(&self) -> u32 {
        if self.x.native.wave64 { 64 } else { 32 }
    }

    /// Raw hardware lane index (`0..63` on Wave64 hosts, `0..31` on Wave32) in `at`.
    pub(super) fn host_lane_index_at(&mut self, at: usize) -> V {
        self.s.flags |= flags::WAVE_OPS;
        self.s.op(
            at,
            "waveGetLaneIndex",
            Attr::ReadOnly,
            self.s.t.i32,
            111,
            &[],
        )
    }

    /// Base offset in the host wave for the caller's 32-lane console half
    /// (`0` for lanes `0..31`, `32` for lanes `32..63`) when running Wave32 in Wave64.
    pub(super) fn half_base_at(&mut self, at: usize) -> V {
        let lane = self.host_lane_index_at(at);
        let k32 = self.s.m.i32(32);
        self.s.m.bin(at, Bin::And, lane, k32)
    }

    /// Wave ballot `(lo, hi)` in `at`. Under `wave32_in_64`, selects the caller's
    /// 32-lane half word into `lo` and returns `0` for `hi`.
    pub(super) fn ballot_at(&mut self, at: usize, bit: V) -> (V, V) {
        let i32t = self.s.t.i32;
        let four = self.s.m.named_struct("dx.types.fouri32", vec![i32t; 4]);
        let r = self
            .s
            .op(at, "waveActiveBallot", Attr::NoUnwind, four, 116, &[bit]);
        self.s.flags |= flags::WAVE_OPS;
        let lo = self.s.m.extract(at, r, 0);
        let hi = self.s.m.extract(at, r, 1);
        if self.wave32_in_64 {
            let host_lane = self.host_lane_index_at(at);
            let k32 = self.s.m.i32(32);
            let upper = self.s.m.cmp(at, pred::IUGE, host_lane, k32);
            let word = self.s.m.select(at, upper, hi, lo);
            (word, self.s.m.i32(0))
        } else {
            (lo, hi)
        }
    }

    /// Splits execution between lower lanes (`0..31`) and upper lanes
    /// (`32..63`) so native wave intrinsics inside `body` operate strictly on
    /// one 32-lane console half with exact divergent `EXEC` tracking.
    fn split_wave_half(&mut self, body: impl Fn(&mut Self, usize) -> V) -> V {
        let head = self.cur;
        let host_lane = self.host_lane_index_at(head);
        let k32 = self.s.m.i32(32);
        let is_upper = self.s.m.cmp(head, pred::IUGE, host_lane, k32);
        let lower_bb = self.s.m.new_block();
        let upper_bb = self.s.m.new_block();
        let merge_bb = self.s.m.new_block();
        self.s.m.terminate(head, Term::CondBr(is_upper, upper_bb, lower_bb));

        self.cur = lower_bb;
        let r_lo = body(self, lower_bb);
        let lower_end = self.cur;
        self.s.m.terminate(lower_end, Term::Br(merge_bb));

        self.cur = upper_bb;
        let r_hi = body(self, upper_bb);
        let upper_end = self.cur;
        self.s.m.terminate(upper_end, Term::Br(merge_bb));

        self.cur = merge_bb;
        let ty = self.s.m.type_of(r_lo);
        let phi = self.s.m.phi(merge_bb, ty);
        self.s.m.set_phi(phi, vec![(r_lo, lower_end), (r_hi, upper_end)]);
        phi
    }

    /// 32-lane reduction inside the caller's 32-lane half of a Wave64 wave.
    pub(super) fn half_wave_reduce_32(&mut self, op: wave::Op, val: V) -> V {
        use wave::Op;
        self.split_wave_half(|this, bb| match op {
            Op::Or | Op::And | Op::Xor => {
                let k = this.s.m.i8(match op {
                    Op::And => 0,
                    Op::Or => 1,
                    _ => 2,
                });
                this.s.op(
                    bb,
                    "waveActiveBit.i32",
                    Attr::NoUnwind,
                    this.s.t.i32,
                    120,
                    &[val, k],
                )
            }
            Op::AddU | Op::AddF | Op::MinU | Op::MaxU | Op::MinI | Op::MaxI | Op::MinF
            | Op::MaxF => {
                let float = matches!(op, Op::AddF | Op::MinF | Op::MaxF);
                let (code, unsigned) = match op {
                    Op::AddU => (0, 1),
                    Op::AddF => (0, 0),
                    Op::MinU => (2, 1),
                    Op::MaxU => (3, 1),
                    Op::MinI | Op::MinF => (2, 0),
                    _ => (3, 0),
                };
                let ty = if float { this.s.t.f32 } else { this.s.t.i32 };
                let sfx = if float { "f32" } else { "i32" };
                let (k, u) = (this.s.m.i8(code), this.s.m.i8(unsigned));
                this.s.op(
                    bb,
                    &format!("waveActiveOp.{sfx}"),
                    Attr::NoUnwind,
                    ty,
                    119,
                    &[val, k, u],
                )
            }
            Op::AddH => unreachable!("AddH is lowered via Shape::Half as AddF"),
        })
    }

    /// 32-lane sum prefix inside the caller's 32-lane half of a Wave64 wave.
    pub(super) fn half_wave_prefix_32(&mut self, op: wave::Op, val: V, inclusive: bool) -> V {
        let float = matches!(op, wave::Op::AddF);
        self.split_wave_half(|this, bb| {
            let ty = if float { this.s.t.f32 } else { this.s.t.i32 };
            let sfx = if float { "f32" } else { "i32" };
            let (k0, k1) = (this.s.m.i8(0), this.s.m.i8(1));
            let r = this.s.op(
                bb,
                &format!("wavePrefixOp.{sfx}"),
                Attr::NoUnwind,
                ty,
                121,
                &[val, k0, k1],
            );
            if inclusive {
                this.s.m.bin(bb, Bin::Add, r, val)
            } else {
                r
            }
        })
    }

    pub(super) fn lane_index(&mut self) -> V {
        if self.is_mesh() {
            // The console's lane: mesh code reads no other lanes, so host
            // waves need not match the console's.
            let f = self.mesh_thread();
            let m = self.s.m.i32(self.wave() - 1);
            return self.s.m.bin(self.s.entry, Bin::And, f, m);
        }
        let lane = self.host_lane_index_at(self.cur);
        if self.wave32_in_64 {
            let k31 = self.s.m.i32(31);
            self.s.m.bin(self.cur, Bin::And, lane, k31)
        } else {
            lane
        }
    }

    /// `x` of lane `src` (in this lane's quad) through quad reads, whose
    /// lane must be a constant.
    pub(super) fn quad_pick(&mut self, x: V, src: V) -> V {
        self.s.flags |= flags::WAVE_OPS;
        let c = self.cur;
        let three = self.s.m.i32(3);
        let q = self.s.m.bin(c, Bin::And, src, three);
        let mut r = x;
        for n in 0..4 {
            let kn = self.s.m.i32(n);
            let read = self.s.op(
                c,
                "quadReadLaneAt.i32",
                Attr::NoUnwind,
                self.s.t.i32,
                122,
                &[x, kn],
            );
            let is = self.s.m.cmp(c, pred::EQ, q, kn);
            r = self.s.m.select(c, is, read, r);
        }
        r
    }

    /// Whether a quad-local read must reach helper lanes: in a pixel program
    /// every quad a triangle edge crosses has some (the console's whole-quad
    /// mode runs them; discarded pixels stay helpers too), and only quad
    /// reads see them on the host (wave reads give 0 from them on AMD: wrong
    /// derivatives of computed values along every triangle edge).
    pub(super) fn helpers_read(&self, quad: bool) -> bool {
        quad && self.s.pixel
    }

    /// `v_readfirstlane_b32`'s result `first` of `own` in a pixel shader:
    /// a helper lane takes its own value instead.
    ///
    /// On the console, whole-quad mode puts helper lanes in EXEC, so a
    /// waterfall loop (read the first lane's value, run the lanes with that
    /// value, take them out of EXEC) serves helper lanes too. On the host,
    /// wave operations leave helper lanes out (DXIL before
    /// `WaveOpsIncludeHelperLanes`): once only helper lanes are left in such
    /// a loop, `WaveReadLaneFirst` reads no lane, no lane matches, and the
    /// loop runs to [`LOOP_LIMIT`] in every such lane: draws of 0.3 to 1.3 s
    /// each, then DEVICE_HUNG. With its own value a helper lane matches in
    /// the first pass and leaves; what a helper lane computes only feeds
    /// derivatives, and its stores are dropped.
    pub(super) fn helper_reads_own(&mut self, own: V, first: V) -> V {
        if !self.s.pixel {
            return first;
        }
        let c = self.cur;
        let helper = self
            .s
            .op(c, "isHelperLane.i1", Attr::ReadOnly, self.s.t.i1, 221, &[]);
        self.s.m.select(c, helper, own, first)
    }

    pub(super) fn read_lane(&mut self, x: V, lane: V) -> V {
        let c = self.cur;
        let lane = if self.wave32_in_64 {
            let k31 = self.s.m.i32(31);
            let in_half = self.s.m.bin(c, Bin::And, lane, k31);
            let base = self.half_base_at(c);
            self.s.m.bin(c, Bin::Or, in_half, base)
        } else {
            lane
        };
        self.s.op(
            c,
            "waveReadLaneAt.i32",
            Attr::NoUnwind,
            self.s.t.i32,
            117,
            &[x, lane],
        )
    }

    /// Byte offset `index * stride + off` for a structured buffer lowered to a
    /// raw `ByteAddressBuffer` / `RWByteAddressBuffer`. Clamps to `0xFFFF_FFFC`
    /// if `index * stride` or `+ off` overflows `u32`, so out-of-bounds sentinel
    /// indices cannot wrap into valid offsets.
    pub(super) fn structured_byte_offset(
        &mut self,
        index: V,
        off: V,
        used: Option<&UsedResource>,
    ) -> Result<V, Why> {
        let stride = used.map_or(0, |u| u.stride);
        if stride < 4 || !stride.is_multiple_of(4) {
            return Err("a structured buffer of unknown stride");
        }
        let c = self.cur;
        let s = self.s.m.i32(stride);
        let row = self.s.m.bin(c, Bin::Mul, index, s);
        let byte = self.s.m.bin(c, Bin::Add, row, off);
        let max_idx = self.s.m.i32(u32::MAX / stride);
        let mul_ovf = self.s.m.cmp(c, pred::IUGT, index, max_idx);
        let add_ovf = self.s.m.cmp(c, pred::IULT, byte, row);
        let oob = self.s.m.bin(c, Bin::Or, mul_ovf, add_ovf);
        let sentinel = self.s.m.i32(0xFFFF_FFFC);
        Ok(self.s.m.select(c, oob, sentinel, byte))
    }
}
