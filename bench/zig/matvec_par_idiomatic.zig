//! Parallel idiomatic variant (std.Thread.Pool, $ISC_THREADS threads).
const std = @import("std");
const c = @import("common.zig");
const p7 = @import("phase7.zig");

const Ctx = struct {
    m: []const f64,
    v: []const f64,
    out: []f64,
    pub fn run(self: Ctx, k: usize, lo: usize, hi: usize) void {
        _ = k;
        const cols = self.v.len;
        for (self.out[lo..hi], lo..) |*o, i| {
            var s: f64 = 0.0;
            for (self.m[i * cols .. (i + 1) * cols], self.v) |p, q| s += p * q;
            o.* = s;
        }
    }
};

noinline fn kernel(m: []const f64, v: []const f64, out: []f64) void {
    p7.chunks(out.len, Ctx{ .m = m, .v = v, .out = out });
}

pub fn main() !void {
    const a = try c.args();
    const m = try c.readF64(a[1]);
    const v = try c.readF64(a[2]);
    if (v.len == 0 or m.len % v.len != 0) return error.SizeMismatch;
    const out = try p7.allocF64(m.len / v.len);
    try c.startPool();
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
