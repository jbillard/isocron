//! Parallel opt variant (std.Thread.Pool, $ISC_THREADS threads).
//! Two passes over contiguous chunks: each thread counts its kept elements;
//! their prefix sums give the rank of the first kept element of every
//! chunk. Then each thread reduces the blocks of kept elements that start in
//! its chunk (reading on into the next chunks to complete the last one), each
//! block result in its own slot: the result does not depend on the number of
//! threads.
const std = @import("std");
const c = @import("common.zig");
const p7 = @import("phase7.zig");

const Count = struct {
    x: []const f64,
    ranks: *[257]usize,
    pub fn run(self: Count, k: usize, lo: usize, hi: usize) void {
        self.ranks[k + 1] = p7.countKept(self.x[lo..hi]);
    }
};

const Reduce = struct {
    x: []const f64,
    ranks: *const [257]usize,
    parts: []f64,
    pub fn run(self: Reduce, k: usize, lo: usize, hi: usize) void {
        _ = hi;
        // blocks first..last start at a rank in ranks[k]..ranks[k + 1]
        const first = (self.ranks[k] + c.block - 1) / c.block;
        const last = (self.ranks[k + 1] + c.block - 1) / c.block;
        if (first < last) _ = p7.keptBlocks(self.x, lo, first * c.block - self.ranks[k], self.parts[first..last]);
    }
};

noinline fn kernel(x: []const f64, parts: []f64) f64 {
    var ranks: [257]usize = undefined;
    ranks[0] = 0;
    p7.chunks(x.len, Count{ .x = x, .ranks = &ranks });
    for (1..c.threads + 1) |k| ranks[k] += ranks[k - 1];
    p7.chunks(x.len, Reduce{ .x = x, .ranks = &ranks, .parts = parts });
    return p7.combine(parts[0 .. (ranks[c.threads] + c.block - 1) / c.block]);
}

pub fn main() !void {
    const a = try c.args();
    const x = try c.readF64(a[1]);
    const parts = try p7.allocF64(p7.maxBlocks(x.len));
    try c.startPool();
    if (c.threads > 256) return error.TooManyThreads;
    var r: f64 = undefined;
    var t = try std.time.Timer.start();
    for (0..c.reps(a[3])) |_| {
        std.mem.doNotOptimizeAway(x.ptr);
        r = kernel(x, parts);
        std.mem.doNotOptimizeAway(&r);
    }
    try c.printNs(t.read());
    try c.writeF64(a[2], &[_]f64{r});
}
