//! Helpers for the kernels of phase 7 (SPEC section 14): moving average,
//! filter then sum, i64 prefix sum, matrix-vector product. The fixed
//! reduction form itself (blockSum, blockDot, fixed, fixedSum) is the one of
//! common.zig; see PHASE7.md for how each optimized variant reaches it.
const std = @import("std");
const builtin = @import("builtin");
const c = @import("common.zig");

const alloc = std.heap.page_allocator;

fn has(f: std.Target.x86.Feature) bool {
    return builtin.cpu.arch == .x86_64 and std.Target.x86.featureSetHas(builtin.cpu.features, f);
}

const avx512 = has(.avx512f);

// ---------------------------------------------------------------------
// buffers

/// An output buffer of n doubles, allocated once before the timed loop and
/// written once so that its pages are mapped before the timing starts.
pub fn allocF64(n: usize) ![]f64 {
    const out = try alloc.alloc(f64, n);
    @memset(out, 0.0);
    return out;
}

pub fn allocI64(n: usize) ![]i64 {
    const out = try alloc.alloc(i64, n);
    @memset(out, 0);
    return out;
}

/// The input of prefix_sum: integers stored as f64, converted to i64.
pub fn toI64(x: []const f64) ![]i64 {
    const out = try alloc.alloc(i64, x.len);
    for (x, out) |v, *o| o.* = @intFromFloat(v);
    return out;
}

/// The output of prefix_sum: the sums converted to f64.
pub fn toF64(x: []const i64) ![]f64 {
    const out = try alloc.alloc(f64, x.len);
    for (x, out) |v, *o| o.* = @floatFromInt(v);
    return out;
}

// ---------------------------------------------------------------------
// threads

/// Runs `ctx.run(k, lo, hi)` for every thread k on its contiguous chunk
/// lo..hi of 0..n, and waits.
pub fn chunks(n: usize, ctx: anytype) void {
    const Job = struct {
        ctx: @TypeOf(ctx),
        n: usize,
        fn work(j: *const @This(), k: usize) void {
            j.ctx.run(k, j.n * k / c.threads, j.n * (k + 1) / c.threads);
        }
    };
    const job = Job{ .ctx = ctx, .n = n };
    c.forEachThread(&job, Job.work);
}

// ---------------------------------------------------------------------
// moving average: out[i] = sum(x[i..i+64]) / 64 in the fixed form

pub const window = 64;

comptime {
    if (window % c.lanes != 0) @compileError("the window must be a multiple of V");
}

/// Windows computed at once, one per SIMD lane.
const wlanes = if (avx512) 8 else 4;

/// Accumulator j of the fixed form after the level d of the tree, for the
/// windows starting at i, i+1, ..., i+len-1 (one per lane). Level V is the
/// state after the loop over the window: acc[j] = e + w[j] + w[j+V] + ...,
/// left to right. Level d < V: acc[j] = acc[j] + acc[j+d] of level 2d.
///
/// The neutral element e = 0.0 is left out here and added once to the root
/// (`windowsAt`): (0 + a) + b and 0 + (a + b) are equal bit for bit (the
/// only case where 0 + a differs from a is a = -0, and then both give b, or
/// +0 when b is a zero too), and so are (0 + p) + (0 + q) and 0 + (p + q).
/// By induction the whole tree equals 0 + (the tree without the zeros).
inline fn node(comptime T: type, x: []const f64, i: usize, comptime d: usize, comptime j: usize) T {
    const len = @typeInfo(T).vector.len;
    if (d == c.lanes) {
        var s: T = x[i + j ..][0..len].*;
        inline for (1..window / c.lanes) |t| {
            const w: T = x[i + j + t * c.lanes ..][0..len].*;
            s += w;
        }
        return s;
    }
    // depth first: few live registers
    return node(T, x, i, 2 * d, j) + node(T, x, i, 2 * d, j + d);
}

inline fn windowsAt(comptime T: type, x: []const f64, i: usize) T {
    // `var`, not `const`: Zig 0.14.1 folds a comptime-known `0.0 + v` into
    // `v` (even in Debug), which is wrong for v = -0.0 (+0 + -0 = +0). A
    // runtime zero keeps the addition; LLVM does not fold it in strict mode.
    var zero: T = @splat(0.0);
    _ = &zero;
    const n: T = @splat(@floatFromInt(window));
    return (zero + node(T, x, i, 1, 0)) / n;
}

/// out[i] for i in lo..hi, x[lo..hi + 63] read; SIMD across windows: lane t
/// of every vector belongs to the window i + t, so each window gets exactly
/// the operations of the fixed form, in the same order.
pub fn windowMeans(x: []const f64, out: []f64, lo: usize, hi: usize) void {
    var i = lo;
    while (i + wlanes <= hi) : (i += wlanes) {
        out[i..][0..wlanes].* = windowsAt(@Vector(wlanes, f64), x, i);
    }
    while (i < hi) : (i += 1) out[i] = windowsAt(@Vector(1, f64), x, i)[0];
}

/// The number of windows of 64 elements in x.
pub fn windows(n: usize) usize {
    return if (n >= window) n - (window - 1) else 0;
}

// ---------------------------------------------------------------------
// filter then sum: the fixed form over the kept elements (> 0.0)

const V8 = @Vector(8, f64);

/// AVX-512 vcompresspd to memory writes only the kept elements. On the test
/// machine (Rocket Lake) it compacts twice as fast as the register form
/// followed by a full 64-byte store, which is limited by the store traffic
/// (0.11 against 0.20 ns per element on the small input; a table of indices
/// with vpermpd is no faster than the register form). The memory form is
/// microcoded and slow on AMD Zen 4 and 5, which keep the register form.
const compress_to_memory = !std.mem.startsWith(u8, builtin.cpu.model.name, "znver");

/// Writes the kept lanes of v (mask m) to dst[0..], packed, in their order.
inline fn compressStore(v: V8, m: u8, dst: [*]f64) void {
    if (compress_to_memory) {
        asm volatile ("vcompresspd %[v], (%[p]) {%[m]}"
            :
            : [v] "v" (v),
              [p] "r" (dst),
              [m] "^Yk" (m),
            : "memory"
        );
    } else {
        dst[0..8].* = asm ("vcompresspd %[v], %[r] {%[m]} {z}"
            : [r] "=v" (-> V8),
            : [v] "v" (v),
              [m] "^Yk" (m),
        );
    }
}

/// Writes the elements > 0.0 of x to dst[0..], in their order, and returns
/// their number. dst needs 8 slots after the last kept element (the vector
/// stores write 8 lanes).
pub fn compact(x: []const f64, dst: [*]f64) usize {
    var n: usize = 0;
    var i: usize = 0;
    if (avx512) {
        const zero: V8 = @splat(0.0);
        while (i + 8 <= x.len) : (i += 8) {
            const v: V8 = x[i..][0..8].*;
            const m: u8 = @bitCast(v > zero);
            compressStore(v, m, dst + n);
            n += @popCount(m);
        }
    }
    // branchless: the store always happens, the count moves on if kept
    while (i < x.len) : (i += 1) {
        dst[n] = x[i];
        n += @intFromBool(x[i] > 0.0);
    }
    return n;
}

/// Input elements compacted at a time.
const piece = 512;

/// Scans x from lo, drops the first `skip` kept elements, and reduces the
/// following ones by blocks of `block` kept elements into parts[0], parts[1],
/// ... (reduce_block of each block); stops when parts is full or at the end
/// of x (the last block may then be shorter). Returns the number of block
/// results written.
pub fn keptBlocks(x: []const f64, lo: usize, skip: usize, parts: []f64) usize {
    var buf: [c.block + piece + 8]f64 align(64) = undefined;
    var n: usize = 0; // kept elements in buf
    var nb: usize = 0;
    var drop = skip;
    var i = lo;
    while (i < x.len and nb < parts.len) {
        const e = @min(i + piece, x.len);
        var k = compact(x[i..e], buf[n..].ptr);
        i = e;
        if (drop > 0) {
            // only at the start, while n == 0
            const d = @min(drop, k);
            std.mem.copyForwards(f64, buf[n .. n + k - d], buf[n + d .. n + k]);
            k -= d;
            drop -= d;
        }
        n += k;
        if (n >= c.block) {
            parts[nb] = c.blockSum(buf[0..c.block]);
            nb += 1;
            std.mem.copyForwards(f64, buf[0 .. n - c.block], buf[c.block..n]);
            n -= c.block;
        }
    }
    if (nb < parts.len and n > 0) {
        parts[nb] = c.blockSum(buf[0..n]);
        nb += 1;
    }
    return nb;
}

/// The fixed form from its block results: reduce_block itself when there is
/// a single block (or none), the fixed form of the block results otherwise.
pub fn combine(parts: []const f64) f64 {
    if (parts.len == 0) return c.blockSum(&[_]f64{});
    if (parts.len == 1) return parts[0];
    return c.fixedSum(parts);
}

/// The block results that a filter of n elements can produce at most.
pub fn maxBlocks(n: usize) usize {
    return @max(1, (n + c.block - 1) / c.block);
}

/// The number of elements > 0.0 (vectorized by LLVM).
pub fn countKept(x: []const f64) usize {
    var n: usize = 0;
    for (x) |v| n += @intFromBool(v > 0.0);
    return n;
}

// ---------------------------------------------------------------------
// prefix sum of i64 (wrapping): explicit SIMD scan

const ilanes = if (avx512) 8 else 4;
const VI = @Vector(ilanes, i64);

/// The inclusive prefix sums of the lanes of v: log2(lanes) shifts and adds.
inline fn scanVec(v: VI) VI {
    var r = v;
    comptime var s = 1;
    inline while (s < ilanes) : (s *= 2) {
        const mask = comptime blk: {
            var m: [ilanes]i32 = undefined;
            for (0..ilanes) |t| m[t] = if (t >= s) @intCast(t - s) else -1;
            break :blk m;
        };
        r +%= @shuffle(i64, r, @as(VI, @splat(0)), mask);
    }
    return r;
}

inline fn broadcastLast(v: VI) VI {
    return @shuffle(i64, v, undefined, [_]i32{ilanes - 1} ** ilanes);
}

/// out[i] = carry + x[0] + ... + x[i] (wrapping); returns the last sum
/// (carry if x is empty). Two vectors per step: the dependency between steps
/// is a single vector addition.
pub fn scanFrom(x: []const i64, out: []i64, carry: i64) i64 {
    var cv: VI = @splat(carry);
    var i: usize = 0;
    while (i + 2 * ilanes <= x.len) : (i += 2 * ilanes) {
        const a = scanVec(x[i..][0..ilanes].*);
        const b = scanVec(x[i + ilanes ..][0..ilanes].*) +% broadcastLast(a);
        out[i..][0..ilanes].* = a +% cv;
        out[i + ilanes ..][0..ilanes].* = b +% cv;
        cv +%= broadcastLast(b);
    }
    var s = cv[0];
    while (i < x.len) : (i += 1) {
        s +%= x[i];
        out[i] = s;
    }
    return s;
}

/// The wrapping sum of x (vectorized by LLVM).
pub fn sumI64(x: []const i64) i64 {
    var s: i64 = 0;
    for (x) |v| s +%= v;
    return s;
}

// ---------------------------------------------------------------------
// matrix-vector product: out[i] = the fixed form of row i times v

const DotCtx = struct {
    x: []const f64,
    y: []const f64,
    pub fn run(self: DotCtx, s: usize, m: usize) f64 {
        return c.blockDot(self.x[s .. s + m], self.y[s .. s + m]);
    }
};

/// out[i] for the rows lo..hi of m (rows of v.len elements).
pub fn rowsDot(m: []const f64, v: []const f64, out: []f64, lo: usize, hi: usize) void {
    const cols = v.len;
    for (lo..hi) |i| out[i] = c.fixed(cols, DotCtx{ .x = m[i * cols ..][0..cols], .y = v });
}
