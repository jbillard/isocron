// Reassociated variant (SPEC section 14): @setFloatMode(.optimized).
const std = @import("std");
const c = @import("common.zig");
const p7 = @import("phase7.zig");

noinline fn kernel(m: []const f64, v: []const f64, out: []f64) void {
    @setFloatMode(.optimized);
    const cols = v.len;
    for (out, 0..) |*o, i| {
        var s: f64 = 0.0;
        for (m[i * cols .. (i + 1) * cols], v) |p, q| s += p * q;
        o.* = s;
    }
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
