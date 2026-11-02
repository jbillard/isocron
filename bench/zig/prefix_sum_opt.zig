const std = @import("std");
const c = @import("common.zig");
const p7 = @import("phase7.zig");

/// SIMD scan: the prefix sums inside each vector by shifts and adds, then
/// the running total added to the whole vector.
noinline fn kernel(x: []const i64, out: []i64) void {
    _ = p7.scanFrom(x, out, 0);
}

pub fn main() !void {
    const a = try c.args();
    const x = try p7.toI64(try c.readF64(a[1]));
    const out = try p7.allocI64(x.len);
    var t = try std.time.Timer.start();
    for (0..c.reps(a[3])) |_| {
        std.mem.doNotOptimizeAway(x.ptr);
        kernel(x, out);
        std.mem.doNotOptimizeAway(out.ptr);
    }
    try c.printNs(t.read());
    try c.writeF64(a[2], try p7.toF64(out));
}
