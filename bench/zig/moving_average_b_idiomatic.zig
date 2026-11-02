//! Comparison B of SPEC section 14: the O(n) algorithm, a running sum (one
//! addition and one subtraction per window instead of 63 additions). Its
//! results differ from the sums of the windows by the rounding accumulated
//! in the running sum.
const std = @import("std");
const c = @import("common.zig");
const p7 = @import("phase7.zig");

noinline fn kernel(x: []const f64, out: []f64) void {
    if (out.len == 0) return;
    var s: f64 = 0.0;
    for (x[0..64]) |v| s += v;
    out[0] = s / 64.0;
    for (1..out.len) |i| {
        s += x[i + 63] - x[i - 1];
        out[i] = s / 64.0;
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
