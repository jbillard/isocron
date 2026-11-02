const std = @import("std");
const c = @import("common.zig");

noinline fn kernel(x: []const f64) f64 {
    return c.fixedSum(x);
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
