const std = @import("std");
const c = @import("common.zig");
const p7 = @import("phase7.zig");

/// The kept elements are compacted into a buffer of one block, each full
/// block reduced as it fills: the fixed form of the filtered array.
noinline fn kernel(x: []const f64, parts: []f64) f64 {
    return p7.combine(parts[0..p7.keptBlocks(x, 0, 0, parts)]);
}

pub fn main() !void {
    const a = try c.args();
    const x = try c.readF64(a[1]);
    const parts = try p7.allocF64(p7.maxBlocks(x.len));
    var r: f64 = 0;
    var t = try std.time.Timer.start();
    for (0..c.reps(a[3])) |_| {
        std.mem.doNotOptimizeAway(x.ptr);
        r = kernel(x, parts);
        std.mem.doNotOptimizeAway(r);
    }
    try c.printNs(t.read());
    try c.writeF64(a[2], &[_]f64{r});
}
