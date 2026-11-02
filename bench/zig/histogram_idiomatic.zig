const std = @import("std");
const c = @import("common.zig");

noinline fn kernel(x: []const u8) [256]u32 {
    var h = [_]u32{0} ** 256;
    for (x) |b| h[b] += 1;
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
