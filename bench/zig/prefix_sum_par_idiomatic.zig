//! Parallel idiomatic variant (std.Thread.Pool, $ISC_THREADS threads).
//! Two passes over contiguous chunks: the sum of each chunk, then each chunk
//! scanned from the sum of the chunks before it.
const std = @import("std");
const c = @import("common.zig");
const p7 = @import("phase7.zig");

const Sums = struct {
    x: []const i64,
    offs: *[257]i64,
    pub fn run(self: Sums, k: usize, lo: usize, hi: usize) void {
        var s: i64 = 0;
        for (self.x[lo..hi]) |v| s +%= v;
        self.offs[k + 1] = s;
    }
};

const Scan = struct {
    x: []const i64,
    out: []i64,
    offs: *const [257]i64,
    pub fn run(self: Scan, k: usize, lo: usize, hi: usize) void {
        var s = self.offs[k];
        for (self.x[lo..hi], self.out[lo..hi]) |v, *o| {
            s +%= v;
            o.* = s;
        }
    }
};

noinline fn kernel(x: []const i64, out: []i64) void {
    var offs: [257]i64 = undefined;
    offs[0] = 0;
    p7.chunks(x.len, Sums{ .x = x, .offs = &offs });
    for (1..c.threads + 1) |k| offs[k] +%= offs[k - 1];
    p7.chunks(x.len, Scan{ .x = x, .out = out, .offs = &offs });
}

pub fn main() !void {
    const a = try c.args();
    const x = try p7.toI64(try c.readF64(a[1]));
    const out = try p7.allocI64(x.len);
    try c.startPool();
    if (c.threads > 256) return error.TooManyThreads;
    var t = try std.time.Timer.start();
    for (0..c.reps(a[3])) |_| {
        std.mem.doNotOptimizeAway(x.ptr);
        kernel(x, out);
        std.mem.doNotOptimizeAway(out.ptr);
    }
    try c.printNs(t.read());
    try c.writeF64(a[2], try p7.toF64(out));
}
