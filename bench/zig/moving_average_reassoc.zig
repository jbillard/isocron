// Reassociated variant (SPEC section 14): @setFloatMode(.optimized).
const std = @import("std");
const c = @import("common.zig");
const p7 = @import("phase7.zig");

noinline fn kernel(x: []const f64, out: []f64) void {
    @setFloatMode(.optimized);
    for (out, 0..) |*o, i| {
        var s: f64 = 0.0;
        for (x[i .. i + 64]) |v| s += v;
        o.* = s / 64.0;
    }
}

pub fn main() !void {
    const a = try c.args();
    const x = try c.readF64(a[1]);
    const out = try p7.allocF64(p7.windows(x.len));
    var t = try std.time.Timer.start();
    for (0..c.reps(a[3])) |_| {
        std.mem.doNotOptimizeAway(x.ptr);
        kernel(x, out);
        std.mem.doNotOptimizeAway(out.ptr);
    }
    try c.printNs(t.read());
    try c.writeF64(a[2], out);
}
