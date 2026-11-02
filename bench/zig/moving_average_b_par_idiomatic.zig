//! Comparison B, parallel (std.Thread.Pool, $ISC_THREADS threads): the
//! running sum of moving_average_b_idiomatic.zig on contiguous chunks of the
//! windows, restarted at the start of each chunk (the results depend on the
//! number of threads).
const std = @import("std");
const c = @import("common.zig");
const p7 = @import("phase7.zig");

const Ctx = struct {
    x: []const f64,
    out: []f64,
    pub fn run(self: Ctx, k: usize, lo: usize, hi: usize) void {
        _ = k;
        if (lo == hi) return;
        var s: f64 = 0.0;
        for (self.x[lo .. lo + 64]) |v| s += v;
        self.out[lo] = s / 64.0;
        for (lo + 1..hi) |i| {
            s += self.x[i + 63] - self.x[i - 1];
            self.out[i] = s / 64.0;
        }
    }
};

noinline fn kernel(x: []const f64, out: []f64) void {
    p7.chunks(out.len, Ctx{ .x = x, .out = out });
}

pub fn main() !void {
    const a = try c.args();
    const x = try c.readF64(a[1]);
    const out = try p7.allocF64(p7.windows(x.len));
    try c.startPool();
    var t = try std.time.Timer.start();
    for (0..c.reps(a[3])) |_| {
        std.mem.doNotOptimizeAway(x.ptr);
        kernel(x, out);
        std.mem.doNotOptimizeAway(out.ptr);
    }
    try c.printNs(t.read());
    try c.writeF64(a[2], out);
}
