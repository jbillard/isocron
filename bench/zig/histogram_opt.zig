const std = @import("std");
const c = @import("common.zig");

/// Four sub-histograms break the store-to-load dependency on repeated values.
noinline fn kernel(x: []const u8) [256]u32 {
    var t = [_][256]u32{[_]u32{0} ** 256} ** 4;
    var i: usize = 0;
    while (i + 4 <= x.len) : (i += 4) {
        t[0][x[i]] += 1;
        t[1][x[i + 1]] += 1;
        t[2][x[i + 2]] += 1;
        t[3][x[i + 3]] += 1;
    }
    while (i < x.len) : (i += 1) t[0][x[i]] += 1;
    var h: [256]u32 = undefined;
    for (0..256) |j| h[j] = t[0][j] + t[1][j] + t[2][j] + t[3][j];
    return h;
}

pub fn main() !void {
    const a = try c.args();
    const x = try c.readU8(a[1]);
    var h: [256]u32 = undefined;
    var t = try std.time.Timer.start();
    for (0..c.reps(a[3])) |_| {
        std.mem.doNotOptimizeAway(x.ptr);
        h = kernel(x);
        std.mem.doNotOptimizeAway(&h);
    }
    try c.printNs(t.read());
    var out: [256]f64 = undefined;
    for (h, 0..) |v, i| out[i] = @floatFromInt(v);
    try c.writeF64(a[2], &out);
}
