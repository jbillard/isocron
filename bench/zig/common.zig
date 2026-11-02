//! Shared I/O, timing and the fixed reduction form of SPEC section 10 for
//! the Zig reference programs. The optimized variants use explicit SIMD:
//! the V accumulators are @Vector(group, f64) values, one pass per group.
const std = @import("std");
const builtin = @import("builtin");

/// V of the fixed form (`lanes.zig`, rewritten for the experiments on V).
pub const lanes = @import("lanes.zig").lanes;
pub const block = 4096;
pub const V = @Vector(lanes, f64);

fn has(f: std.Target.x86.Feature) bool {
    return builtin.cpu.arch == .x86_64 and std.Target.x86.featureSetHas(builtin.cpu.features, f);
}

/// Accumulators per pass over a block (speed only, never results), the rule
/// of Isocron's code generator: the accumulators of one pass, plus one
/// register per array read (one less with AVX, which folds a load into the
/// addition), must fit in the vector registers. group1: one array read per
/// element, group2: two. Each accumulator still receives its elements in
/// increasing order.
pub const group1 = @min(lanes, if (has(.avx)) 64 else 16);
pub const group2 = @min(lanes, if (has(.avx512f)) 64 else if (has(.avx)) 32 else 16);

const alloc = std.heap.page_allocator;

pub fn args() ![][:0]u8 {
    return std.process.argsAlloc(alloc);
}

pub fn readU8(path: []const u8) ![]u8 {
    return std.fs.cwd().readFileAlloc(alloc, path, 1 << 34);
}

pub fn readF64(path: []const u8) ![]f64 {
    const bytes = try std.fs.cwd().readFileAllocOptions(alloc, path, 1 << 34, null, @alignOf(f64), null);
    return std.mem.bytesAsSlice(f64, bytes);
}

pub fn writeF64(path: []const u8, x: []const f64) !void {
    const out = try alloc.alloc(u8, x.len * 8);
    for (x, 0..) |v, i| {
        const bits: u64 = if (std.math.isNan(v)) 0x7FF8000000000000 else @bitCast(v);
        std.mem.writeInt(u64, out[i * 8 ..][0..8], bits, .little);
    }
    try std.fs.cwd().writeFile(.{ .sub_path = path, .data = out });
}

pub fn reps(s: []const u8) u64 {
    const r = std.fmt.parseInt(u64, s, 10) catch 1;
    return if (r < 1) 1 else r;
}

pub fn printNs(ns: u64) !void {
    try std.io.getStdOut().writer().print("{d}\n", .{ns});
}

fn tree(acc: [lanes]f64) f64 {
    var a = acc;
    var d: usize = lanes / 2;
    while (d >= 1) : (d /= 2) {
        for (0..d) |j| a[j] = a[j] + a[j + d];
    }
    return a[0];
}

/// reduce_block with + over x.
pub fn blockSum(x: []const f64) f64 {
    const G = @Vector(group1, f64);
    var t: [lanes]f64 = undefined;
    const full = x.len / lanes * lanes;
    inline for (0..lanes / group1) |p| {
        var acc: G = @splat(0.0);
        var i: usize = 0;
        while (i < full) : (i += lanes) {
            const v: G = x[i + p * group1 ..][0..group1].*;
            acc += v;
        }
        t[p * group1 ..][0..group1].* = acc;
    }
    for (x[full..], 0..) |v, j| t[j] += v;
    return tree(t);
}

/// reduce_block with + over a[i] * b[i].
pub fn blockDot(a: []const f64, b: []const f64) f64 {
    const G = @Vector(group2, f64);
    var t: [lanes]f64 = undefined;
    const full = a.len / lanes * lanes;
    inline for (0..lanes / group2) |p| {
        var acc: G = @splat(0.0);
        var i: usize = 0;
        while (i < full) : (i += lanes) {
            const va: G = a[i + p * group2 ..][0..group2].*;
            const vb: G = b[i + p * group2 ..][0..group2].*;
            acc += va * vb;
        }
        t[p * group2 ..][0..group2].* = acc;
    }
    for (a[full..], b[full..], 0..) |x, y, j| t[j] += x * y;
    return tree(t);
}

/// reduce_block with + over (x[i] - m)^2.
pub fn blockSq(x: []const f64, m: f64) f64 {
    const G = @Vector(group1, f64);
    var t: [lanes]f64 = undefined;
    const full = x.len / lanes * lanes;
    const mv: G = @splat(m);
    inline for (0..lanes / group1) |p| {
        var acc: G = @splat(0.0);
        var i: usize = 0;
        while (i < full) : (i += lanes) {
            const v: G = x[i + p * group1 ..][0..group1].*;
            const d = v - mv;
            acc += d * d;
        }
        t[p * group1 ..][0..group1].* = acc;
    }
    for (x[full..], 0..) |v, j| {
        const d = v - m;
        t[j] += d * d;
    }
    return tree(t);
}

/// The fixed form: blocks of `block` elements, then the block results are
/// summed the same way. `ctx.run(start, m)` reduces one block.
pub fn fixed(n: usize, ctx: anytype) f64 {
    if (n <= block) return ctx.run(0, n);
    const c = (n + block - 1) / block;
    // A stack buffer up to 4096 partial results (n <= 16.7 million): the page
    // allocator would make two system calls per call.
    var buf: [block]f64 = undefined;
    const heap = c > block;
    const parts = if (heap) alloc.alloc(f64, c) catch unreachable else buf[0..c];
    defer if (heap) alloc.free(parts);
    for (0..c) |j| parts[j] = ctx.run(j * block, @min(block, n - j * block));
    return fixedSum(parts);
}

const SumCtx = struct {
    x: []const f64,
    pub fn run(self: SumCtx, s: usize, m: usize) f64 {
        return blockSum(self.x[s .. s + m]);
    }
};

pub fn fixedSum(x: []const f64) f64 {
    return fixed(x.len, SumCtx{ .x = x });
}

// ---------------------------------------------------------------------
// parallel variants (SPEC section 14: Zig uses std.Thread)

pub var pool: std.Thread.Pool = undefined;
pub var threads: usize = 1;

/// Starts a persistent pool of `$ISC_THREADS` threads (default: the number
/// of CPUs), before the timed loop.
pub fn startPool() !void {
    threads = blk: {
        if (std.posix.getenv("ISC_THREADS")) |s| break :blk std.fmt.parseInt(usize, s, 10) catch 1;
        break :blk std.Thread.getCpuCount() catch 1;
    };
    if (threads < 1) threads = 1;
    try pool.init(.{ .allocator = alloc, .n_jobs = threads });
}

/// Runs `f(ctx, k)` for every thread k in 0..threads and waits.
pub fn forEachThread(ctx: anytype, comptime f: fn (@TypeOf(ctx), usize) void) void {
    var wg: std.Thread.WaitGroup = .{};
    for (0..threads) |k| pool.spawnWg(&wg, f, .{ ctx, k });
    pool.waitAndWork(&wg);
}

/// The fixed form with the blocks computed in parallel: thread k computes
/// blocks k, k + T, ...; each block result has its own slot, so the result
/// does not depend on the number of threads.
pub fn fixedPar(n: usize, ctx: anytype) f64 {
    if (n <= block) return ctx.run(0, n);
    const c = (n + block - 1) / block;
    var buf: [block]f64 = undefined;
    const heap = c > block;
    const parts = if (heap) alloc.alloc(f64, c) catch unreachable else buf[0..c];
    defer if (heap) alloc.free(parts);
    const Job = struct {
        ctx: @TypeOf(ctx),
        parts: []f64,
        n: usize,
        fn work(j: *const @This(), k: usize) void {
            var b = k;
            while (b < j.parts.len) : (b += threads) j.parts[b] = j.ctx.run(b * block, @min(block, j.n - b * block));
        }
    };
    const job = Job{ .ctx = ctx, .parts = parts, .n = n };
    forEachThread(&job, Job.work);
    return fixedSum(parts);
}

/// One contiguous chunk per thread, summed from left to right, then the
/// partial sums in order (the result depends on the number of threads).
pub fn chunkSums(n: usize, ctx: anytype) f64 {
    var parts: [256]f64 = undefined;
    const Job = struct {
        ctx: @TypeOf(ctx),
        parts: *[256]f64,
        n: usize,
        fn work(j: *const @This(), k: usize) void {
            j.parts[k] = j.ctx.run(j.n * k / threads, j.n * (k + 1) / threads);
        }
    };
    const job = Job{ .ctx = ctx, .parts = &parts, .n = n };
    forEachThread(&job, Job.work);
    var s: f64 = 0;
    for (parts[0..threads]) |p| s += p;
    return s;
}

/// Adds to h the four sub-histograms of x.
pub fn subHistograms(x: []const u8, h: *[256]u32) void {
    var t = [_][256]u32{[_]u32{0} ** 256} ** 4;
    var i: usize = 0;
    while (i + 4 <= x.len) : (i += 4) {
        t[0][x[i]] += 1;
        t[1][x[i + 1]] += 1;
        t[2][x[i + 2]] += 1;
        t[3][x[i + 3]] += 1;
    }
    while (i < x.len) : (i += 1) t[0][x[i]] += 1;
    for (0..256) |j| h[j] += t[0][j] + t[1][j] + t[2][j] + t[3][j];
}

/// One histogram per thread on a contiguous chunk, merged.
pub fn histogramPar(x: []const u8, comptime sub: bool) [256]u32 {
    var hs: [256][256]u32 = undefined;
    const Job = struct {
        x: []const u8,
        hs: *[256][256]u32,
        fn work(j: *const @This(), k: usize) void {
            const c = j.x[j.x.len * k / threads .. j.x.len * (k + 1) / threads];
            j.hs[k] = [_]u32{0} ** 256;
            if (sub) {
                subHistograms(c, &j.hs[k]);
            } else {
                for (c) |b| j.hs[k][b] += 1;
            }
        }
    };
    const job = Job{ .x = x, .hs = &hs };
    forEachThread(&job, Job.work);
    var h = [_]u32{0} ** 256;
    for (0..threads) |k| {
        for (0..256) |j| h[j] += hs[k][j];
    }
    return h;
}
