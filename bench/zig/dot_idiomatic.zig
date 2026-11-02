const std = @import("std");
const c = @import("common.zig");

noinline fn kernel(x: []const f64, y: []const f64) f64 {
    var s: f64 = 0.0;
    for (x, y) |p, q| s += p * q;
    return s;
}

pub fn main() !void {
    const a = try c.args();
    const x = try c.readF64(a[1]);
    const y = try c.readF64(a[2]);
    if (x.len != y.len) return error.SizeMismatch;
    var r: f64 = 0;
    var t = try std.time.Timer.start();
    for (0..c.reps(a[4])) |_| {
        std.mem.doNotOptimizeAway(x.ptr);
        std.mem.doNotOptimizeAway(y.ptr);
        r = kernel(x, y);
        std.mem.doNotOptimizeAway(r);
    }
    try c.printNs(t.read());
    try c.writeF64(a[3], &[_]f64{r});
}
