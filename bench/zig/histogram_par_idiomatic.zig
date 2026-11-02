//! Parallel idiomatic variant (std.Thread.Pool, $ISC_THREADS threads).
const std = @import("std");
const c = @import("common.zig");

noinline fn kernel(x: []const u8) [256]u32 {
    return c.histogramPar(x, false);
}

pub fn main() !void {
    const a = try c.args();
    const x = try c.readU8(a[1]);
    try c.startPool();
    var r: [256]u32 = undefined;
    var t = try std.time.Timer.start();
    for (0..c.reps(a[3])) |_| {
        std.mem.doNotOptimizeAway(x.ptr);
        r = kernel(x);
        std.mem.doNotOptimizeAway(&r);
    }
    try c.printNs(t.read());
    var out: [256]f64 = undefined;
    for (r, 0..) |v, i| out[i] = @floatFromInt(v);
    try c.writeF64(a[2], &out);
}
