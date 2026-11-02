const std = @import("std");
const c = @import("common.zig");
const p7 = @import("phase7.zig");

/// Each row: the fixed form of its products with v (blockDot of common.zig).
noinline fn kernel(m: []const f64, v: []const f64, out: []f64) void {
    p7.rowsDot(m, v, out, 0, out.len);
}

pub fn main() !void {
    const a = try c.args();
    const m = try c.readF64(a[1]);
    const v = try c.readF64(a[2]);
    if (v.len == 0 or m.len % v.len != 0) return error.SizeMismatch;
    const out = try p7.allocF64(m.len / v.len);
    var t = try std.time.Timer.start();
    for (0..c.reps(a[4])) |_| {
        std.mem.doNotOptimizeAway(m.ptr);
        std.mem.doNotOptimizeAway(v.ptr);
        kernel(m, v, out);
        std.mem.doNotOptimizeAway(out.ptr);
    }
    try c.printNs(t.read());
    try c.writeF64(a[3], out);
}
