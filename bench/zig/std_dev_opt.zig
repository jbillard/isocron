const std = @import("std");
const c = @import("common.zig");

const Ctx = struct {
    x: []const f64,
    m: f64,
    pub fn run(self: Ctx, s: usize, k: usize) f64 {
        return c.blockSq(self.x[s .. s + k], self.m);
    }
};

noinline fn kernel(x: []const f64) f64 {
    const n: f64 = @floatFromInt(x.len);
    const m = c.fixedSum(x) / n;
    return @sqrt(c.fixed(x.len, Ctx{ .x = x, .m = m }) / n);
}

pub fn main() !void {
    const a = try c.args();
    const x = try c.readF64(a[1]);
    var r: f64 = 0;
    var t = try std.time.Timer.start();
    for (0..c.reps(a[3])) |_| {
        std.mem.doNotOptimizeAway(x.ptr);
        r = kernel(x);
        std.mem.doNotOptimizeAway(r);
    }
    try c.printNs(t.read());
    try c.writeF64(a[2], &[_]f64{r});
}
