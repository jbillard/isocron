//! The module `unix` (D632): its primitives realized by the compiler. Each
//! is a function of the module (`define internal`), written once in the
//! program that calls it, whose calls of the system are the instruction
//! `syscall` of Linux x86-64: no function of the runtime (D622). The
//! interpreter makes the same calls (`isc_interp::unix`).
//!
//! The codes are those of the runtime (`code_of` of `system.rs`): 0, the
//! kinds 1 to 6 of `IoError`, or the error of the kernel negated.

use super::{v, Gen, V};
use isc_ir::SysOp;

/// A call of the kernel: its number, then six arguments (`rdi`, `rsi`,
/// `rdx`, `r10`, `r8`, `r9`); the result in `rax`, `rcx` and `r11`
/// clobbered, memory read and written.
const SYSCALL: &str = "asm sideeffect \"syscall\", \"={rax},{rax},{rdi},{rsi},{rdx},{r10},{r8},{r9},~{rcx},~{r11},~{memory}\"";

/// The result of a call of the kernel as a code; `again`: `EAGAIN` the
/// code 6 of a time limit passed (`SO_RCVTIMEO`), as `net_code` of the
/// runtime.
const CODE: &str = r#"define internal i64 @isc.unix.code(i64 %r, i1 %again) {
entry:
  %ok = icmp sge i64 %r, 0
  br i1 %ok, label %zero, label %err
zero:
  ret i64 0
err:
  %e = sub i64 0, %r
  %eagain = icmp eq i64 %e, 11
  %late = and i1 %eagain, %again
  br i1 %late, label %c6, label %which
which:
  switch i64 %e, label %other [ i64 2, label %c1
                                i64 1, label %c2
                                i64 13, label %c2
                                i64 17, label %c3
                                i64 98, label %c3
                                i64 39, label %c4
                                i64 111, label %c5
                                i64 110, label %c6 ]
c1:
  ret i64 1
c2:
  ret i64 2
c3:
  ret i64 3
c4:
  ret i64 4
c5:
  ret i64 5
c6:
  ret i64 6
other:
  ret i64 %r
}"#;

/// The address of a Unix socket (`sockaddr_un`, 110 bytes) at `sa` for the
/// `n` bytes at `p`: a path (at most 107 bytes, no zero byte), or `@` and
/// a name of the abstract space of Linux (at most 107 bytes, the `@`
/// written as the zero byte that marks it); its length, 0 when invalid.
const ADDR: &str = r#"define internal i64 @isc.unix.addr(ptr %sa, ptr %p, i64 %n) {
entry:
  store i16 1, ptr %sa
  %path = getelementptr i8, ptr %sa, i64 2
  %empty = icmp eq i64 %n, 0
  br i1 %empty, label %bad, label %first
first:
  %c0 = load i8, ptr %p
  %abstract = icmp eq i8 %c0, 64
  br i1 %abstract, label %name, label %file
name:
  %m = sub i64 %n, 1
  %mlong = icmp ugt i64 %m, 107
  br i1 %mlong, label %bad, label %ncopy
ncopy:
  store i8 0, ptr %path
  %ndst = getelementptr i8, ptr %sa, i64 3
  %nsrc = getelementptr i8, ptr %p, i64 1
  call void @llvm.memcpy.p0.p0.i64(ptr %ndst, ptr %nsrc, i64 %m, i1 false)
  %nlen = add i64 %n, 2
  ret i64 %nlen
file:
  %flong = icmp ugt i64 %n, 107
  br i1 %flong, label %bad, label %scan
scan:
  %i = phi i64 [ 0, %file ], [ %i1, %byte ]
  %end = icmp eq i64 %i, %n
  br i1 %end, label %fcopy, label %byte
byte:
  %q = getelementptr i8, ptr %p, i64 %i
  %b = load i8, ptr %q
  %zero = icmp eq i8 %b, 0
  %i1 = add i64 %i, 1
  br i1 %zero, label %bad, label %scan
fcopy:
  call void @llvm.memcpy.p0.p0.i64(ptr %path, ptr %p, i64 %n, i1 false)
  %nul = getelementptr i8, ptr %path, i64 %n
  store i8 0, ptr %nul
  %flen = add i64 %n, 3
  ret i64 %flen
bad:
  ret i64 0
}"#;

/// `sysUnixConnect(path)` and `sysUnixListen(path)` (`listen`): a socket
/// of stream (`SOCK_CLOEXEC`) connected, or bound and listening (128
/// waiting); `[socket, code]`, -1 on an error, the socket then closed.
const SOCKET: &str = r#"define internal { i64, i64 } @isc.unix.socket(ptr %p, i64 %n, i1 %listen) {
entry:
  %sa = alloca [112 x i8], align 8
  %len = call i64 @isc.unix.addr(ptr %sa, ptr %p, i64 %n)
  %invalid = icmp eq i64 %len, 0
  br i1 %invalid, label %inval, label %open
inval:
  ret { i64, i64 } { i64 -1, i64 -22 }
open:
  %fd = call i64 @SC(i64 41, i64 1, i64 524289, i64 0, i64 0, i64 0, i64 0)
  %nofd = icmp slt i64 %fd, 0
  br i1 %nofd, label %fail0, label %what
fail0:
  %c0 = call i64 @isc.unix.code(i64 %fd, i1 false)
  %r0 = insertvalue { i64, i64 } { i64 -1, i64 0 }, i64 %c0, 1
  ret { i64, i64 } %r0
what:
  br i1 %listen, label %bind, label %connect
connect:
  %cr = call i64 @SC(i64 42, i64 %fd, ptr %sa, i64 %len, i64 0, i64 0, i64 0)
  %cintr = icmp eq i64 %cr, -4
  br i1 %cintr, label %connect, label %connected
connected:
  %cbad = icmp slt i64 %cr, 0
  br i1 %cbad, label %close, label %ok
bind:
  %br = call i64 @SC(i64 49, i64 %fd, ptr %sa, i64 %len, i64 0, i64 0, i64 0)
  %bbad = icmp slt i64 %br, 0
  br i1 %bbad, label %close, label %listens
listens:
  %lr = call i64 @SC(i64 50, i64 %fd, i64 128, i64 0, i64 0, i64 0, i64 0)
  %lbad = icmp slt i64 %lr, 0
  br i1 %lbad, label %close, label %ok
close:
  %e = phi i64 [ %cr, %connected ], [ %br, %bind ], [ %lr, %listens ]
  %x = call i64 @SC(i64 3, i64 %fd, i64 0, i64 0, i64 0, i64 0, i64 0)
  %c1 = call i64 @isc.unix.code(i64 %e, i1 false)
  %r1 = insertvalue { i64, i64 } { i64 -1, i64 0 }, i64 %c1, 1
  ret { i64, i64 } %r1
ok:
  %r2 = insertvalue { i64, i64 } { i64 0, i64 0 }, i64 %fd, 0
  ret { i64, i64 } %r2
}"#;

/// `sysUnixAccept(listener)`: `accept4` (`SOCK_CLOEXEC`), again after a
/// signal; `[conn, code]`.
const ACCEPT: &str = r#"define internal { i64, i64 } @isc.unix.accept(i64 %l) {
entry:
  br label %again
again:
  %fd = call i64 @SC(i64 288, i64 %l, i64 0, i64 0, i64 524288, i64 0, i64 0)
  %intr = icmp eq i64 %fd, -4
  br i1 %intr, label %again, label %done
done:
  %bad = icmp slt i64 %fd, 0
  br i1 %bad, label %fail, label %ok
fail:
  %c = call i64 @isc.unix.code(i64 %fd, i1 true)
  %r0 = insertvalue { i64, i64 } { i64 -1, i64 0 }, i64 %c, 1
  ret { i64, i64 } %r0
ok:
  %r1 = insertvalue { i64, i64 } { i64 0, i64 0 }, i64 %fd, 0
  ret { i64, i64 } %r1
}"#;

/// `sysSendFds(conn, bytes, fds)`: the `m` descriptors at `f` with the
/// first part of the `n` bytes at `p` (`sendmsg`, `SCM_RIGHTS`), the rest
/// by `sendto`; `MSG_NOSIGNAL` (a peer gone is an error, not `SIGPIPE`).
/// Descriptors go with at least one byte, at most 253 of them
/// (`SCM_MAX_FD`): -22 otherwise. The padding after them (to 8 bytes) is
/// written zeros first: the kernel reads it.
const SEND_FDS: &str = r#"define internal i64 @isc.unix.sendfds(i64 %c, ptr %p, i64 %n, ptr %f, i64 %m) {
entry:
  %ctl = alloca [1040 x i8], align 8
  %msg = alloca [56 x i8], align 8
  %iov = alloca [16 x i8], align 8
  %nofds = icmp eq i64 %m, 0
  br i1 %nofds, label %plain, label %check
check:
  %many = icmp ugt i64 %m, 253
  %nodata = icmp eq i64 %n, 0
  %invalid = or i1 %many, %nodata
  br i1 %invalid, label %inval, label %build
inval:
  ret i64 -22
build:
  store ptr %p, ptr %iov
  %iovlen = getelementptr i8, ptr %iov, i64 8
  store i64 %n, ptr %iovlen
  store ptr null, ptr %msg
  %mname = getelementptr i8, ptr %msg, i64 8
  store i64 0, ptr %mname
  %miov = getelementptr i8, ptr %msg, i64 16
  store ptr %iov, ptr %miov
  %miovlen = getelementptr i8, ptr %msg, i64 24
  store i64 1, ptr %miovlen
  %fb = shl i64 %m, 2
  %clen = add i64 %fb, 16
  %clen7 = add i64 %clen, 7
  %space = and i64 %clen7, -8
  %lastat = sub i64 %space, 8
  %last = getelementptr i8, ptr %ctl, i64 %lastat
  store i64 0, ptr %last
  store i64 %clen, ptr %ctl
  %level = getelementptr i8, ptr %ctl, i64 8
  store i32 1, ptr %level
  %type = getelementptr i8, ptr %ctl, i64 12
  store i32 1, ptr %type
  %data = getelementptr i8, ptr %ctl, i64 16
  call void @llvm.memcpy.p0.p0.i64(ptr %data, ptr %f, i64 %fb, i1 false)
  %mctl = getelementptr i8, ptr %msg, i64 32
  store ptr %ctl, ptr %mctl
  %mctllen = getelementptr i8, ptr %msg, i64 40
  store i64 %space, ptr %mctllen
  %mflags = getelementptr i8, ptr %msg, i64 48
  store i64 0, ptr %mflags
  br label %send
send:
  %r = call i64 @SC(i64 46, i64 %c, ptr %msg, i64 16384, i64 0, i64 0, i64 0)
  %intr = icmp eq i64 %r, -4
  br i1 %intr, label %send, label %sent
sent:
  %rbad = icmp slt i64 %r, 0
  br i1 %rbad, label %fail, label %rest
plain:
  br label %rest
rest:
  %from = phi i64 [ 0, %plain ], [ %r, %sent ]
  br label %loop
loop:
  %at = phi i64 [ %from, %rest ], [ %next, %more ]
  %done = icmp sge i64 %at, %n
  br i1 %done, label %ok, label %write
write:
  %q = getelementptr i8, ptr %p, i64 %at
  %left = sub i64 %n, %at
  %w = call i64 @SC(i64 44, i64 %c, ptr %q, i64 %left, i64 16384, i64 0, i64 0)
  %wintr = icmp eq i64 %w, -4
  br i1 %wintr, label %write, label %written
written:
  %wbad = icmp slt i64 %w, 0
  br i1 %wbad, label %fail, label %more
more:
  %next = add i64 %at, %w
  br label %loop
fail:
  %e = phi i64 [ %r, %sent ], [ %w, %written ]
  %code = call i64 @isc.unix.code(i64 %e, i1 true)
  ret i64 %code
ok:
  ret i64 0
}"#;

/// `sysRecvFds(conn, max, maxFds)`: one `recvmsg` (`MSG_CMSG_CLOEXEC`) of
/// at most `max` bytes (1 to 16 MiB) and `maxFds` descriptors (0 to 253);
/// `[bytes, fds, code]`. The bytes in a block of their length when they
/// fill less than half of `max` (as `bytes_read` of the runtime). More
/// descriptors than `maxFds` (`MSG_CTRUNC`): those received closed, the
/// bytes dropped, the code of `EMSGSIZE`.
const RECV_FDS: &str = r#"define internal { { ptr, i64 }, { ptr, i64 }, i64 } @isc.unix.recvfds(i64 %c, i64 %max, i64 %maxf) {
entry:
  %ctl = alloca [1040 x i8], align 8
  %msg = alloca [56 x i8], align 8
  %iov = alloca [16 x i8], align 8
  %low = icmp slt i64 %max, 1
  %m1 = select i1 %low, i64 1, i64 %max
  %high = icmp sgt i64 %m1, 16777216
  %mx = select i1 %high, i64 16777216, i64 %m1
  %fneg = icmp slt i64 %maxf, 0
  %f0 = select i1 %fneg, i64 0, i64 %maxf
  %fhigh = icmp sgt i64 %f0, 253
  %mf = select i1 %fhigh, i64 253, i64 %f0
  %buf = call ptr @isc_alloc(i64 %mx)
  store ptr %buf, ptr %iov
  %iovlen = getelementptr i8, ptr %iov, i64 8
  store i64 %mx, ptr %iovlen
  store ptr null, ptr %msg
  %mname = getelementptr i8, ptr %msg, i64 8
  store i64 0, ptr %mname
  %miov = getelementptr i8, ptr %msg, i64 16
  store ptr %iov, ptr %miov
  %miovlen = getelementptr i8, ptr %msg, i64 24
  store i64 1, ptr %miovlen
  %fb = shl i64 %mf, 2
  %cl = add i64 %fb, 23
  %space0 = and i64 %cl, -8
  %nofds = icmp eq i64 %mf, 0
  %space = select i1 %nofds, i64 0, i64 %space0
  %mctl = getelementptr i8, ptr %msg, i64 32
  store ptr %ctl, ptr %mctl
  %mctllen = getelementptr i8, ptr %msg, i64 40
  store i64 %space, ptr %mctllen
  %mflags = getelementptr i8, ptr %msg, i64 48
  store i64 0, ptr %mflags
  br label %recv
recv:
  %r = call i64 @SC(i64 47, i64 %c, ptr %msg, i64 1073741824, i64 0, i64 0, i64 0)
  %intr = icmp eq i64 %r, -4
  br i1 %intr, label %recv, label %got
got:
  %rbad = icmp slt i64 %r, 0
  br i1 %rbad, label %fail, label %fds
fail:
  call void @isc_free(ptr %buf)
  %code = call i64 @isc.unix.code(i64 %r, i1 true)
  %eb = call ptr @isc_alloc(i64 1)
  %ef = call ptr @isc_alloc(i64 4)
  %a0 = insertvalue { ptr, i64 } { ptr null, i64 0 }, ptr %eb, 0
  %b0 = insertvalue { ptr, i64 } { ptr null, i64 0 }, ptr %ef, 0
  %t0 = insertvalue { { ptr, i64 }, { ptr, i64 }, i64 } poison, { ptr, i64 } %a0, 0
  %t1 = insertvalue { { ptr, i64 }, { ptr, i64 }, i64 } %t0, { ptr, i64 } %b0, 1
  %t2 = insertvalue { { ptr, i64 }, { ptr, i64 }, i64 } %t1, i64 %code, 2
  ret { { ptr, i64 }, { ptr, i64 }, i64 } %t2
fds:
  %rcl = load i64, ptr %mctllen
  %flags = load i32, ptr %mflags
  %fb0 = icmp eq i64 %fb, 0
  %osz = select i1 %fb0, i64 4, i64 %fb
  %out = call ptr @isc_alloc(i64 %osz)
  br label %walk
walk:
  %off = phi i64 [ 0, %fds ], [ %noff, %step ]
  %k = phi i64 [ 0, %fds ], [ %k2, %step ]
  %h = add i64 %off, 16
  %fits = icmp ule i64 %h, %rcl
  br i1 %fits, label %one, label %walked
one:
  %cp = getelementptr i8, ptr %ctl, i64 %off
  %len = load i64, ptr %cp
  %lp = getelementptr i8, ptr %cp, i64 8
  %lev = load i32, ptr %lp
  %tp = getelementptr i8, ptr %cp, i64 12
  %ty = load i32, ptr %tp
  %islev = icmp eq i32 %lev, 1
  %isty = icmp eq i32 %ty, 1
  %rights = and i1 %islev, %isty
  %short = icmp ult i64 %len, 16
  br i1 %short, label %walked, label %which
which:
  br i1 %rights, label %take, label %skip
take:
  %nb = sub i64 %len, 16
  %cnt = lshr i64 %nb, 2
  %room = sub i64 %mf, %k
  %over = icmp ugt i64 %cnt, %room
  %cnt2 = select i1 %over, i64 %room, i64 %cnt
  %dst = getelementptr i32, ptr %out, i64 %k
  %src = getelementptr i8, ptr %cp, i64 16
  %bytes = shl i64 %cnt2, 2
  call void @llvm.memcpy.p0.p0.i64(ptr %dst, ptr %src, i64 %bytes, i1 false)
  %k1 = add i64 %k, %cnt2
  br label %step
skip:
  br label %step
step:
  %k2 = phi i64 [ %k1, %take ], [ %k, %skip ]
  %l7 = add i64 %len, 7
  %al = and i64 %l7, -8
  %noff = add i64 %off, %al
  br label %walk
walked:
  %trunc = and i32 %flags, 8
  %lost = icmp ne i32 %trunc, 0
  br i1 %lost, label %close, label %done
close:
  %j = phi i64 [ 0, %walked ], [ %j1, %closeone ]
  %closed = icmp eq i64 %j, %k
  br i1 %closed, label %dropped, label %closeone
closeone:
  %fp = getelementptr i32, ptr %out, i64 %j
  %fd32 = load i32, ptr %fp
  %fd = sext i32 %fd32 to i64
  %x = call i64 @SC(i64 3, i64 %fd, i64 0, i64 0, i64 0, i64 0, i64 0)
  %j1 = add i64 %j, 1
  br label %close
dropped:
  call void @isc_free(ptr %buf)
  %db = call ptr @isc_alloc(i64 1)
  %a1 = insertvalue { ptr, i64 } { ptr null, i64 0 }, ptr %db, 0
  %b1 = insertvalue { ptr, i64 } { ptr null, i64 0 }, ptr %out, 0
  %u0 = insertvalue { { ptr, i64 }, { ptr, i64 }, i64 } poison, { ptr, i64 } %a1, 0
  %u1 = insertvalue { { ptr, i64 }, { ptr, i64 }, i64 } %u0, { ptr, i64 } %b1, 1
  %u2 = insertvalue { { ptr, i64 }, { ptr, i64 }, i64 } %u1, i64 -90, 2
  ret { { ptr, i64 }, { ptr, i64 }, i64 } %u2
done:
  %r2 = shl i64 %r, 1
  %keep = icmp sge i64 %r2, %mx
  br i1 %keep, label %fin, label %copy
copy:
  %rzero = icmp eq i64 %r, 0
  %sz = select i1 %rzero, i64 1, i64 %r
  %qb = call ptr @isc_alloc(i64 %sz)
  call void @llvm.memcpy.p0.p0.i64(ptr %qb, ptr %buf, i64 %r, i1 false)
  call void @isc_free(ptr %buf)
  br label %fin
fin:
  %bp = phi ptr [ %buf, %done ], [ %qb, %copy ]
  %a2 = insertvalue { ptr, i64 } undef, ptr %bp, 0
  %a3 = insertvalue { ptr, i64 } %a2, i64 %r, 1
  %b2 = insertvalue { ptr, i64 } undef, ptr %out, 0
  %b3 = insertvalue { ptr, i64 } %b2, i64 %k, 1
  %v0 = insertvalue { { ptr, i64 }, { ptr, i64 }, i64 } poison, { ptr, i64 } %a3, 0
  %v1 = insertvalue { { ptr, i64 }, { ptr, i64 }, i64 } %v0, { ptr, i64 } %b3, 1
  %v2 = insertvalue { { ptr, i64 }, { ptr, i64 }, i64 } %v1, i64 0, 2
  ret { { ptr, i64 }, { ptr, i64 }, i64 } %v2
}"#;

/// `sysPoll(fds, ms)`: `poll` of the `n` descriptors at `f` for reading
/// (`POLLIN`), `ms` ms at most (below 0: no limit), again after a signal;
/// `[events, code]`, the events of each its `revents` (all 0 on an error).
/// The `pollfd` of at most 64 descriptors on the stack (a block of the heap
/// at each call made the wait of one descriptor ready 14 % slower than its
/// floor).
const POLL: &str = r#"define internal { { ptr, i64 }, i64 } @isc.unix.poll(ptr %f, i64 %n, i64 %ms) {
entry:
  %stack = alloca [64 x i64], align 8
  %none = icmp eq i64 %n, 0
  %small = icmp ule i64 %n, 64
  br i1 %small, label %start, label %heap
heap:
  %sz8 = shl i64 %n, 3
  %hp = call ptr @isc_alloc(i64 %sz8)
  br label %start
start:
  %pf = phi ptr [ %stack, %entry ], [ %hp, %heap ]
  br label %fill
fill:
  %i = phi i64 [ 0, %start ], [ %i1, %put ]
  %filled = icmp eq i64 %i, %n
  br i1 %filled, label %call, label %put
put:
  %src = getelementptr i32, ptr %f, i64 %i
  %fd = load i32, ptr %src
  %fd64 = zext i32 %fd to i64
  %word = or i64 %fd64, 4294967296
  %dst = getelementptr i64, ptr %pf, i64 %i
  store i64 %word, ptr %dst
  %i1 = add i64 %i, 1
  br label %fill
call:
  %neg = icmp slt i64 %ms, 0
  %big = icmp sgt i64 %ms, 2147483647
  %t1 = select i1 %big, i64 2147483647, i64 %ms
  %t = select i1 %neg, i64 -1, i64 %t1
  br label %again
again:
  %r = call i64 @SC(i64 7, ptr %pf, i64 %n, i64 %t, i64 0, i64 0, i64 0)
  %intr = icmp eq i64 %r, -4
  br i1 %intr, label %again, label %polled
polled:
  %code = call i64 @isc.unix.code(i64 %r, i1 false)
  %sz4 = shl i64 %n, 2
  %osz = select i1 %none, i64 4, i64 %sz4
  %out = call ptr @isc_alloc(i64 %osz)
  br label %read
read:
  %j = phi i64 [ 0, %polled ], [ %j1, %get ]
  %got = icmp eq i64 %j, %n
  br i1 %got, label %done, label %get
get:
  %wp = getelementptr i64, ptr %pf, i64 %j
  %w = load i64, ptr %wp
  %ev = lshr i64 %w, 48
  %ev32 = trunc i64 %ev to i32
  %op = getelementptr i32, ptr %out, i64 %j
  store i32 %ev32, ptr %op
  %j1 = add i64 %j, 1
  br label %read
done:
  br i1 %small, label %freed, label %free
free:
  call void @isc_free(ptr %pf)
  br label %freed
freed:
  %a0 = insertvalue { ptr, i64 } undef, ptr %out, 0
  %a1 = insertvalue { ptr, i64 } %a0, i64 %n, 1
  %v0 = insertvalue { { ptr, i64 }, i64 } poison, { ptr, i64 } %a1, 0
  %v1 = insertvalue { { ptr, i64 }, i64 } %v0, i64 %code, 1
  ret { { ptr, i64 }, i64 } %v1
}"#;

/// `sysCloseFd(fd)`.
const CLOSE_FD: &str = r#"define internal i64 @isc.unix.closefd(i32 %fd) {
entry:
  %fd64 = sext i32 %fd to i64
  %r = call i64 @SC(i64 3, i64 %fd64, i64 0, i64 0, i64 0, i64 0, i64 0)
  %c = call i64 @isc.unix.code(i64 %r, i1 false)
  ret i64 %c
}"#;

/// `sysWriteFd(fd, bytes)` (D650): `write` until all is written, again
/// after a signal (`EINTR`), the descriptor waited for while it is full
/// (`EAGAIN`, `poll` for `POLLOUT`). `SIGPIPE` blocked for the thread
/// during the call (`rt_sigprocmask`): a reader gone is `EPIPE`, the signal
/// the write raised then taken from those pending (`rt_sigtimedwait`
/// without waiting), the mask of before set again; the program never ended
/// by it (a pipe whose reader has gone, unlike a socket, has no
/// `MSG_NOSIGNAL`).
const WRITE_FD: &str = r#"define internal i64 @isc.unix.writefd(i32 %fd, ptr %p, i64 %n) {
entry:
  %set = alloca i64, align 8
  %old = alloca i64, align 8
  %ts = alloca [2 x i64], align 8
  %pfd = alloca [2 x i32], align 8
  %atp = alloca i64, align 8
  %fd64 = sext i32 %fd to i64
  store i64 4096, ptr %set
  store i64 0, ptr %old
  store i64 0, ptr %atp
  %blocked = call i64 @SC(i64 14, i64 0, ptr %set, ptr %old, i64 8, i64 0, i64 0)
  br label %loop
loop:
  %at = load i64, ptr %atp
  %done = icmp sge i64 %at, %n
  br i1 %done, label %all, label %write
write:
  %q = getelementptr i8, ptr %p, i64 %at
  %left = sub i64 %n, %at
  %w = call i64 @SC(i64 1, i64 %fd64, ptr %q, i64 %left, i64 0, i64 0, i64 0)
  %wintr = icmp eq i64 %w, -4
  br i1 %wintr, label %loop, label %written
written:
  %full = icmp eq i64 %w, -11
  br i1 %full, label %wait, label %checked
wait:
  store i32 %fd, ptr %pfd
  %ev = getelementptr i8, ptr %pfd, i64 4
  store i32 4, ptr %ev
  %pr = call i64 @SC(i64 7, ptr %pfd, i64 1, i64 -1, i64 0, i64 0, i64 0)
  br label %loop
checked:
  %wbad = icmp slt i64 %w, 0
  br i1 %wbad, label %finish, label %more
more:
  %next = add i64 %at, %w
  store i64 %next, ptr %atp
  br label %loop
all:
  br label %finish
finish:
  %r = phi i64 [ 0, %all ], [ %w, %checked ]
  %pipe = icmp eq i64 %r, -32
  br i1 %pipe, label %take, label %restore
take:
  store i64 0, ptr %ts
  %ns = getelementptr i8, ptr %ts, i64 8
  store i64 0, ptr %ns
  %taken = call i64 @SC(i64 128, ptr %set, i64 0, ptr %ts, i64 8, i64 0, i64 0)
  br label %restore
restore:
  %unblocked = call i64 @SC(i64 14, i64 2, ptr %old, i64 0, i64 8, i64 0, i64 0)
  %c = call i64 @isc.unix.code(i64 %r, i1 false)
  ret i64 %c
}"#;

/// The name of the files of `sysSharedOpen` (seen in `/proc/<pid>/fd`).
const SHARED_NAME: &str = r#"@isc.unix.memfd = private unnamed_addr constant [8 x i8] c"isocron\00""#;

/// A memory shared (D632): a block of 32 bytes of the runtime's allocator,
/// its address the handle: the address of the projection, its bytes, the
/// descriptor of its file (-1 when it is the caller's, `sysSharedMap`),
/// and whether it can be written.
///
/// `sysSharedOpen(bytes)`: `memfd_create` (`MFD_CLOEXEC`), `ftruncate`,
/// `mmap` shared, readable and writable; `[shared, code]`.
const SHARED_OPEN: &str = r#"define internal { i64, i64 } @isc.unix.shared_open(i64 %bytes) {
entry:
  %empty = icmp slt i64 %bytes, 1
  br i1 %empty, label %inval, label %make
inval:
  ret { i64, i64 } { i64 -1, i64 -22 }
make:
  %fd = call i64 @SC(i64 319, ptr @isc.unix.memfd, i64 1, i64 0, i64 0, i64 0, i64 0)
  %nofd = icmp slt i64 %fd, 0
  br i1 %nofd, label %fail0, label %size
fail0:
  %c0 = call i64 @isc.unix.code(i64 %fd, i1 false)
  %r0 = insertvalue { i64, i64 } { i64 -1, i64 0 }, i64 %c0, 1
  ret { i64, i64 } %r0
size:
  %t = call i64 @SC(i64 77, i64 %fd, i64 %bytes, i64 0, i64 0, i64 0, i64 0)
  %tbad = icmp slt i64 %t, 0
  br i1 %tbad, label %close, label %map
map:
  %a = call i64 @SC(i64 9, i64 0, i64 %bytes, i64 3, i64 1, i64 %fd, i64 0)
  %abad = icmp ugt i64 %a, -4096
  br i1 %abad, label %close, label %ok
close:
  %e = phi i64 [ %t, %size ], [ %a, %map ]
  %x = call i64 @SC(i64 3, i64 %fd, i64 0, i64 0, i64 0, i64 0, i64 0)
  %c1 = call i64 @isc.unix.code(i64 %e, i1 false)
  %r1 = insertvalue { i64, i64 } { i64 -1, i64 0 }, i64 %c1, 1
  ret { i64, i64 } %r1
ok:
  %blk = call ptr @isc_alloc(i64 32)
  store i64 %a, ptr %blk
  %pl = getelementptr i8, ptr %blk, i64 8
  store i64 %bytes, ptr %pl
  %pf = getelementptr i8, ptr %blk, i64 16
  store i64 %fd, ptr %pf
  %pw = getelementptr i8, ptr %blk, i64 24
  store i64 1, ptr %pw
  %h = ptrtoint ptr %blk to i64
  %r2 = insertvalue { i64, i64 } { i64 0, i64 0 }, i64 %h, 0
  ret { i64, i64 } %r2
}"#;

/// `sysSharedMap(fd, bytes, writable)`: `mmap` of the file `fd`, shared,
/// readable and writable, or private and read only; `[shared, code]`.
const SHARED_MAP: &str = r#"define internal { i64, i64 } @isc.unix.shared_map(i32 %fd, i64 %bytes, i1 %w) {
entry:
  %empty = icmp slt i64 %bytes, 1
  br i1 %empty, label %inval, label %map
inval:
  ret { i64, i64 } { i64 -1, i64 -22 }
map:
  %fd64 = sext i32 %fd to i64
  %prot = select i1 %w, i64 3, i64 1
  %flags = select i1 %w, i64 1, i64 2
  %a = call i64 @SC(i64 9, i64 0, i64 %bytes, i64 %prot, i64 %flags, i64 %fd64, i64 0)
  %abad = icmp ugt i64 %a, -4096
  br i1 %abad, label %fail, label %ok
fail:
  %c = call i64 @isc.unix.code(i64 %a, i1 false)
  %r0 = insertvalue { i64, i64 } { i64 -1, i64 0 }, i64 %c, 1
  ret { i64, i64 } %r0
ok:
  %blk = call ptr @isc_alloc(i64 32)
  store i64 %a, ptr %blk
  %pl = getelementptr i8, ptr %blk, i64 8
  store i64 %bytes, ptr %pl
  %pf = getelementptr i8, ptr %blk, i64 16
  store i64 -1, ptr %pf
  %pw = getelementptr i8, ptr %blk, i64 24
  %w64 = zext i1 %w to i64
  store i64 %w64, ptr %pw
  %h = ptrtoint ptr %blk to i64
  %r1 = insertvalue { i64, i64 } { i64 0, i64 0 }, i64 %h, 0
  ret { i64, i64 } %r1
}"#;

/// `sysSharedWrite(shared, at, bytes)` (and `sysSharedWrite32`, its words
/// counted in bytes by the caller): -22 past the end, the code 2 (denied)
/// for a projection read only.
const SHARED_WRITE: &str = r#"define internal i64 @isc.unix.shared_write(i64 %h, i64 %at, ptr %p, i64 %n) {
entry:
  %blk = inttoptr i64 %h to ptr
  %pw = getelementptr i8, ptr %blk, i64 24
  %w = load i64, ptr %pw
  %ro = icmp eq i64 %w, 0
  br i1 %ro, label %denied, label %bounds
denied:
  ret i64 2
bounds:
  %pl = getelementptr i8, ptr %blk, i64 8
  %len = load i64, ptr %pl
  %neg = icmp slt i64 %at, 0
  %over = icmp ugt i64 %n, %len
  %room = sub i64 %len, %n
  %past = icmp ugt i64 %at, %room
  %b0 = or i1 %neg, %over
  %bad = or i1 %b0, %past
  br i1 %bad, label %inval, label %copy
inval:
  ret i64 -22
copy:
  %a = load ptr, ptr %blk
  %dst = getelementptr i8, ptr %a, i64 %at
  call void @llvm.memcpy.p0.p0.i64(ptr %dst, ptr %p, i64 %n, i1 false)
  ret i64 0
}"#;

/// `sysSharedRead(shared, at, n)`: `[bytes, code]`, none and -22 past the
/// end.
const SHARED_READ: &str = r#"define internal { { ptr, i64 }, i64 } @isc.unix.shared_read(i64 %h, i64 %at, i64 %n) {
entry:
  %blk = inttoptr i64 %h to ptr
  %pl = getelementptr i8, ptr %blk, i64 8
  %len = load i64, ptr %pl
  %neg = icmp slt i64 %at, 0
  %nneg = icmp slt i64 %n, 0
  %over = icmp ugt i64 %n, %len
  %room = sub i64 %len, %n
  %past = icmp ugt i64 %at, %room
  %b0 = or i1 %neg, %nneg
  %b1 = or i1 %b0, %over
  %bad = or i1 %b1, %past
  br i1 %bad, label %inval, label %copy
inval:
  %e = call ptr @isc_alloc(i64 1)
  %a0 = insertvalue { ptr, i64 } { ptr null, i64 0 }, ptr %e, 0
  %v0 = insertvalue { { ptr, i64 }, i64 } poison, { ptr, i64 } %a0, 0
  %v1 = insertvalue { { ptr, i64 }, i64 } %v0, i64 -22, 1
  ret { { ptr, i64 }, i64 } %v1
copy:
  %zero = icmp eq i64 %n, 0
  %sz = select i1 %zero, i64 1, i64 %n
  %out = call ptr @isc_alloc(i64 %sz)
  %a = load ptr, ptr %blk
  %src = getelementptr i8, ptr %a, i64 %at
  call void @llvm.memcpy.p0.p0.i64(ptr %out, ptr %src, i64 %n, i1 false)
  %a1 = insertvalue { ptr, i64 } undef, ptr %out, 0
  %a2 = insertvalue { ptr, i64 } %a1, i64 %n, 1
  %v2 = insertvalue { { ptr, i64 }, i64 } poison, { ptr, i64 } %a2, 0
  %v3 = insertvalue { { ptr, i64 }, i64 } %v2, i64 0, 1
  ret { { ptr, i64 }, i64 } %v3
}"#;

/// `sysSharedClose(shared)`: `munmap`, its file closed when it is its own,
/// the block freed; the code of `munmap`. The handle of a failure (-1):
/// nothing to close.
const SHARED_CLOSE: &str = r#"define internal i64 @isc.unix.shared_close(i64 %h) {
entry:
  %none = icmp eq i64 %h, -1
  br i1 %none, label %nothing, label %some
nothing:
  ret i64 0
some:
  %blk = inttoptr i64 %h to ptr
  %a = load i64, ptr %blk
  %pl = getelementptr i8, ptr %blk, i64 8
  %len = load i64, ptr %pl
  %pf = getelementptr i8, ptr %blk, i64 16
  %fd = load i64, ptr %pf
  %r = call i64 @SC(i64 11, i64 %a, i64 %len, i64 0, i64 0, i64 0, i64 0)
  %own = icmp sge i64 %fd, 0
  br i1 %own, label %close, label %done
close:
  %x = call i64 @SC(i64 3, i64 %fd, i64 0, i64 0, i64 0, i64 0, i64 0)
  br label %done
done:
  call void @isc_free(ptr %blk)
  %c = call i64 @isc.unix.code(i64 %r, i1 false)
  ret i64 %c
}"#;

/// The text of a function above, its calls of the kernel written out.
fn ir(text: &str) -> String {
    text.replace("call i64 @SC(", &format!("call i64 {}(", SYSCALL))
}

impl Gen<'_> {
    /// The functions `defs` of this module defined in the program, once
    /// each: with the functions of the program, so that their blocks come
    /// from the allocator inlined into it as theirs (D605; among the
    /// declarations, they called the runtime's: the wait for one descriptor
    /// 12 % slower than its floor). A global (`@…`) goes with the
    /// declarations.
    fn unix_defs(&mut self, defs: &[&str]) {
        for d in [CODE].iter().chain(defs) {
            if d.starts_with('@') {
                self.decls.insert(d.to_string());
                continue;
            }
            let name = d.split('@').nth(1).and_then(|r| r.split('(').next()).unwrap_or(d);
            if self.decls.insert(format!("; the module unix: @{}", name)) {
                self.funcs.push(format!("{}\n", ir(d)));
            }
        }
    }

    /// A primitive of the module `unix` (D632), its arguments evaluated.
    pub(super) fn unix_op(&mut self, o: SysOp, vals: &[V]) -> V {
        match o {
            SysOp::UnixConnect | SysOp::UnixListen => {
                self.unix_defs(&[ADDR, SOCKET]);
                let (p, n) = self.str_parts(&vals[0]);
                let listen = o == SysOp::UnixListen;
                v(self.ins(format!("call {{ i64, i64 }} @isc.unix.socket(ptr {}, i64 {}, i1 {})", p, n, listen)), "{ i64, i64 }")
            }
            SysOp::UnixAccept => {
                self.unix_defs(&[ACCEPT]);
                v(self.ins(format!("call {{ i64, i64 }} @isc.unix.accept(i64 {})", vals[0].r)), "{ i64, i64 }")
            }
            SysOp::SendFds => {
                self.unix_defs(&[SEND_FDS]);
                let (p, n) = self.str_parts(&vals[1]);
                let (f, m) = self.str_parts(&vals[2]);
                v(self.ins(format!("call i64 @isc.unix.sendfds(i64 {}, ptr {}, i64 {}, ptr {}, i64 {})", vals[0].r, p, n, f, m)), "i64")
            }
            SysOp::RecvFds => {
                self.unix_defs(&[RECV_FDS]);
                let t = "{ { ptr, i64 }, { ptr, i64 }, i64 }";
                v(self.ins(format!("call {} @isc.unix.recvfds(i64 {}, i64 {}, i64 {})", t, vals[0].r, vals[1].r, vals[2].r)), t)
            }
            // a connection is its socket (D339)
            SysOp::ConnFd => vals[0].clone(),
            SysOp::Poll => {
                self.unix_defs(&[POLL]);
                let (f, n) = self.str_parts(&vals[0]);
                let t = "{ { ptr, i64 }, i64 }";
                v(self.ins(format!("call {} @isc.unix.poll(ptr {}, i64 {}, i64 {})", t, f, n, vals[1].r)), t)
            }
            SysOp::CloseFd => {
                self.unix_defs(&[CLOSE_FD]);
                v(self.ins(format!("call i64 @isc.unix.closefd(i32 {})", vals[0].r)), "i64")
            }
            SysOp::WriteFd => {
                self.unix_defs(&[WRITE_FD]);
                let (p, n) = self.str_parts(&vals[1]);
                v(self.ins(format!("call i64 @isc.unix.writefd(i32 {}, ptr {}, i64 {})", vals[0].r, p, n)), "i64")
            }
            SysOp::SharedOpen => {
                self.unix_defs(&[SHARED_NAME, SHARED_OPEN]);
                v(self.ins(format!("call {{ i64, i64 }} @isc.unix.shared_open(i64 {})", vals[0].r)), "{ i64, i64 }")
            }
            SysOp::SharedMap => {
                self.unix_defs(&[SHARED_MAP]);
                v(self.ins(format!("call {{ i64, i64 }} @isc.unix.shared_map(i32 {}, i64 {}, i1 {})", vals[0].r, vals[1].r, vals[2].r)), "{ i64, i64 }")
            }
            SysOp::SharedWrite | SysOp::SharedWrite32 => {
                self.unix_defs(&[SHARED_WRITE]);
                let (p, n) = self.str_parts(&vals[2]);
                let bytes = if o == SysOp::SharedWrite32 { self.ins(format!("shl i64 {}, 2", n)) } else { n };
                v(self.ins(format!("call i64 @isc.unix.shared_write(i64 {}, i64 {}, ptr {}, i64 {})", vals[0].r, vals[1].r, p, bytes)), "i64")
            }
            SysOp::SharedRead => {
                self.unix_defs(&[SHARED_READ]);
                let t = "{ { ptr, i64 }, i64 }";
                v(self.ins(format!("call {} @isc.unix.shared_read(i64 {}, i64 {}, i64 {})", t, vals[0].r, vals[1].r, vals[2].r)), t)
            }
            SysOp::SharedFd | SysOp::SharedSize => {
                let at = if o == SysOp::SharedFd { 16 } else { 8 };
                let blk = self.ins(format!("inttoptr i64 {} to ptr", vals[0].r));
                let p = self.ins(format!("getelementptr i8, ptr {}, i64 {}", blk, at));
                v(self.ins(format!("load i64, ptr {}", p)), "i64")
            }
            SysOp::SharedClose => {
                self.unix_defs(&[SHARED_CLOSE]);
                v(self.ins(format!("call i64 @isc.unix.shared_close(i64 {})", vals[0].r)), "i64")
            }
            _ => unreachable!("not a primitive of the module unix: {:?}", o),
        }
    }
}
