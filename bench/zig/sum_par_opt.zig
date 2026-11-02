//! Parallel opt variant (std.Thread.Pool, $ISC_THREADS threads).
const std = @import("std");
const c = @import("common.zig");

const Ctx = struct {
    x: []const f64,
    pub fn run(self: Ctx, s: usize, m: usize) f64 {
        return c.blockSum(self.x[s .. s + m]);
    }
};

noinline fn kernel(x: []const f64) f64 {
    return c.fixedPar(x.len, Ctx{ .x = x });
}

pub fn main() !void {
    const a = try c.args();
    const x = try c.readF64(a[1]);
    try c.startPool();
    var r: f64 = undefined;
    var t = try std.time.Timer.start();
    for (0..c.reps(a[3])) |_| {
        std.mem.doNotOptimizeAway(x.ptr);
        r = kernel(x);
        std.mem.doNotOptimizeAway(&r);
    }
    try c.printNs(t.read());
    try c.writeF64(a[2], &[_]f64{r});
}
