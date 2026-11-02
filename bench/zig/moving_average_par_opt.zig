//! Parallel opt variant (std.Thread.Pool, $ISC_THREADS threads).
const std = @import("std");
const c = @import("common.zig");
const p7 = @import("phase7.zig");

const Ctx = struct {
    x: []const f64,
    out: []f64,
    pub fn run(self: Ctx, k: usize, lo: usize, hi: usize) void {
        _ = k;
        p7.windowMeans(self.x, self.out, lo, hi);
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
