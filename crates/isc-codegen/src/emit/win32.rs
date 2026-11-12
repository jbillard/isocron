//! The module `win32` (D636): a window of Windows, its messages and the
//! image it shows; its primitives realized by the compiler. Each is a
//! function of the program (`define internal`) that calls `user32`, `gdi32`
//! and `kernel32` (the ABI of Windows x64, the convention of the target),
//! linked by their import libraries: no function of the runtime (D622).
//!
//! Each window has a thread of its own (D641): `sysWinOpen` starts it, it
//! makes the window and reads its messages (`GetMessageW`) until the
//! window is closed. Windows calls the window procedure, a function of the
//! program too, on that thread, even while it holds a loop of its own (a
//! window resized or moved by the hand): the procedure puts each event the
//! program needs in the queue of the window, four words (its kind, then
//! three numbers), under a lock, and signals it; `sysWinEvents`, on the
//! thread of the program, waits for that signal and takes the queue. So
//! the program draws again while the window is resized. The procedure
//! paints the last image shown again itself (`sysWinShow` keeps a copy),
//! the part of the window it does not cover in the colour of its last
//! pixel: no part left black. A window is a block of the runtime's
//! allocator, its address the handle: the window (`HWND`), its queue, the
//! number of events in it, its room, the lock, the signals, the thread,
//! the last image.

use super::{v, Gen, V};
use isc_ir::SysOp;

/// The functions of Windows the module calls.
const DECLS: [&str; 40] = [
    "declare i32 @PeekMessageW(ptr, ptr, i32, i32, i32)",
    "declare ptr @SetCursor(ptr)",
    "declare i32 @GetCursorPos(ptr)",
    "declare ptr @WindowFromPoint(i64)",
    "declare ptr @GetModuleHandleW(ptr)",
    "declare ptr @LoadCursorW(ptr, ptr)",
    "declare i16 @RegisterClassExW(ptr)",
    "declare i32 @AdjustWindowRectEx(ptr, i32, i32, i32)",
    "declare ptr @CreateWindowExW(i32, ptr, ptr, i32, i32, i32, i32, i32, ptr, ptr, ptr, ptr)",
    "declare i32 @ShowWindow(ptr, i32)",
    "declare i32 @UpdateWindow(ptr)",
    "declare i32 @DestroyWindow(ptr)",
    "declare i64 @SetWindowLongPtrW(ptr, i32, i64)",
    "declare i64 @GetWindowLongPtrW(ptr, i32)",
    "declare i64 @DefWindowProcW(ptr, i32, i64, i64)",
    "declare i32 @GetMessageW(ptr, ptr, i32, i32)",
    "declare i32 @TranslateMessage(ptr)",
    "declare i64 @DispatchMessageW(ptr)",
    "declare void @PostQuitMessage(i32)",
    "declare i32 @PostMessageW(ptr, i32, i64, i64)",
    "declare i32 @GetClientRect(ptr, ptr)",
    "declare ptr @BeginPaint(ptr, ptr)",
    "declare i32 @EndPaint(ptr, ptr)",
    "declare ptr @GetDC(ptr)",
    "declare i32 @ReleaseDC(ptr, ptr)",
    "declare i32 @SetDIBitsToDevice(ptr, i32, i32, i32, i32, i32, i32, i32, i32, ptr, ptr, i32)",
    "declare ptr @CreateSolidBrush(i32)",
    "declare i32 @FillRect(ptr, ptr, ptr)",
    "declare i32 @DeleteObject(ptr)",
    "declare i32 @SetWindowPos(ptr, ptr, i32, i32, i32, i32, i32)",
    "declare i32 @MultiByteToWideChar(i32, i32, ptr, i32, ptr, i32)",
    "declare i32 @GetLastError()",
    "declare i32 @SetProcessDpiAwarenessContext(ptr)",
    "declare ptr @CreateThread(ptr, i64, ptr, ptr, i32, ptr)",
    "declare i32 @WaitForSingleObject(ptr, i32)",
    "declare ptr @CreateEventW(ptr, i32, i32, ptr)",
    "declare i32 @SetEvent(ptr)",
    "declare i32 @CloseHandle(ptr)",
    "declare void @InitializeCriticalSection(ptr)",
    "declare void @EnterCriticalSection(ptr)",
];

/// The functions of the lock, beside those above.
const LOCK_DECLS: [&str; 2] = ["declare void @LeaveCriticalSection(ptr)", "declare void @DeleteCriticalSection(ptr)"];

/// The functions of `gdi32` for a capture.
const CAPTURE_DECLS: [&str; 6] = [
    "declare ptr @CreateCompatibleDC(ptr)",
    "declare ptr @CreateCompatibleBitmap(ptr, i32, i32)",
    "declare ptr @SelectObject(ptr, ptr)",
    "declare i32 @BitBlt(ptr, i32, i32, i32, i32, ptr, i32, i32, i32)",
    "declare i32 @GetDIBits(ptr, ptr, i32, i32, ptr, ptr, i32)",
    "declare i32 @DeleteDC(ptr)",
];

// The block of a window, its parts (bytes from its start): the window,
// its queue, the number of events in it, its room; the lock (a
// `CRITICAL_SECTION`, 40 bytes); the signal of an event put, the thread,
// the signal of the window made, the title (UTF-16) and the size of the
// window to make, the code of a failure to make it; the last image shown
// (its pixels, width, height, the room of its block); the signal of a
// flush (`WM_APP + 1` seen by the procedure); the shape of the pointer over
// its inside (the number of a cursor of the system, D646).
// (176 bytes; `isc_alloc` in `OPEN`)

/// The name of the class of the windows, in UTF-16.
const CLASS: &str = r#"@isc.win.class = private unnamed_addr constant [8 x i16] [i16 105, i16 115, i16 111, i16 99, i16 114, i16 111, i16 110, i16 0]"#;

/// An event put in the queue of the window `blk`, under its lock, then
/// signalled: its kind and three numbers (the queue full: dropped).
const PUSH: &str = r#"define internal void @isc.win.push(ptr %blk, i64 %kind, i64 %a, i64 %b, i64 %c) {
entry:
  %cs = getelementptr i8, ptr %blk, i64 32
  call void @EnterCriticalSection(ptr %cs)
  %pq = getelementptr i8, ptr %blk, i64 8
  %q = load ptr, ptr %pq
  %pn = getelementptr i8, ptr %blk, i64 16
  %n = load i64, ptr %pn
  %pcap = getelementptr i8, ptr %blk, i64 24
  %cap = load i64, ptr %pcap
  %full = icmp sge i64 %n, %cap
  br i1 %full, label %done, label %put
put:
  %at = shl i64 %n, 2
  %p0 = getelementptr i64, ptr %q, i64 %at
  store i64 %kind, ptr %p0
  %p1 = getelementptr i64, ptr %p0, i64 1
  store i64 %a, ptr %p1
  %p2 = getelementptr i64, ptr %p0, i64 2
  store i64 %b, ptr %p2
  %p3 = getelementptr i64, ptr %p0, i64 3
  store i64 %c, ptr %p3
  %n1 = add i64 %n, 1
  store i64 %n1, ptr %pn
  br label %done
done:
  call void @LeaveCriticalSection(ptr %cs)
  %pe = getelementptr i8, ptr %blk, i64 72
  %ev = load ptr, ptr %pe
  %set = call i32 @SetEvent(ptr %ev)
  ret void
}"#;

/// The last image shown painted again on `dc` (under the lock: the thread
/// of the program may replace it), the part of the inside it does not
/// cover in the colour of its last pixel (white without an image).
const REPAINT: &str = r#"define internal void @isc.win.repaint(ptr %blk, ptr %hwnd, ptr %dc) {
entry:
  %bmi = alloca [44 x i8], align 8
  %rect = alloca [16 x i8], align 8
  %cl = alloca [16 x i8], align 8
  %cs = getelementptr i8, ptr %blk, i64 32
  call void @EnterCriticalSection(ptr %cs)
  store i64 0, ptr %cl
  %cl8 = getelementptr i8, ptr %cl, i64 8
  store i64 0, ptr %cl8
  %okr = call i32 @GetClientRect(ptr %hwnd, ptr %cl)
  %cw = load i32, ptr %cl8
  %cl12 = getelementptr i8, ptr %cl, i64 12
  %ch = load i32, ptr %cl12
  %pimg = getelementptr i8, ptr %blk, i64 120
  %img = load ptr, ptr %pimg
  %piw = getelementptr i8, ptr %blk, i64 128
  %iw = load i64, ptr %piw
  %pih = getelementptr i8, ptr %blk, i64 136
  %ih = load i64, ptr %pih
  %has = icmp ne ptr %img, null
  br i1 %has, label %draw, label %blank
draw:
  %iw32 = trunc i64 %iw to i32
  %ih32 = trunc i64 %ih to i32
  %top = sub i32 0, %ih32
  store i32 40, ptr %bmi
  %pw = getelementptr i8, ptr %bmi, i64 4
  store i32 %iw32, ptr %pw
  %ph = getelementptr i8, ptr %bmi, i64 8
  store i32 %top, ptr %ph
  %pplanes = getelementptr i8, ptr %bmi, i64 12
  store i16 1, ptr %pplanes
  %pbits = getelementptr i8, ptr %bmi, i64 14
  store i16 32, ptr %pbits
  %prest = getelementptr i8, ptr %bmi, i64 16
  store i64 0, ptr %prest
  %prest2 = getelementptr i8, ptr %bmi, i64 24
  store i64 0, ptr %prest2
  %prest3 = getelementptr i8, ptr %bmi, i64 32
  store i64 0, ptr %prest3
  %prest4 = getelementptr i8, ptr %bmi, i64 40
  store i32 0, ptr %prest4
  %lines = call i32 @SetDIBitsToDevice(ptr %dc, i32 0, i32 0, i32 %iw32, i32 %ih32, i32 0, i32 0, i32 0, i32 %ih32, ptr %img, ptr %bmi, i32 0)
  %all = mul i64 %iw, %ih
  %li = sub i64 %all, 1
  %pp = getelementptr i32, ptr %img, i64 %li
  %px = load i32, ptr %pp
  %r0 = lshr i32 %px, 16
  %r = and i32 %r0, 255
  %g0 = lshr i32 %px, 8
  %g1 = and i32 %g0, 255
  %g = shl i32 %g1, 8
  %b0 = and i32 %px, 255
  %b = shl i32 %b0, 16
  %rg = or i32 %r, %g
  %rgb = or i32 %rg, %b
  br label %strips
blank:
  br label %strips
strips:
  %color = phi i32 [ %rgb, %draw ], [ 16777215, %blank ]
  %w0 = phi i32 [ %iw32, %draw ], [ 0, %blank ]
  %h0 = phi i32 [ %ih32, %draw ], [ 0, %blank ]
  %brush = call ptr @CreateSolidBrush(i32 %color)
  %wide = icmp sgt i32 %cw, %w0
  br i1 %wide, label %right, label %under
right:
  store i32 %w0, ptr %rect
  %rt = getelementptr i8, ptr %rect, i64 4
  store i32 0, ptr %rt
  %rr = getelementptr i8, ptr %rect, i64 8
  store i32 %cw, ptr %rr
  %rb = getelementptr i8, ptr %rect, i64 12
  store i32 %ch, ptr %rb
  %f1 = call i32 @FillRect(ptr %dc, ptr %rect, ptr %brush)
  br label %under
under:
  %tall = icmp sgt i32 %ch, %h0
  br i1 %tall, label %below, label %end
below:
  store i32 0, ptr %rect
  %bt = getelementptr i8, ptr %rect, i64 4
  store i32 %h0, ptr %bt
  %br = getelementptr i8, ptr %rect, i64 8
  store i32 %w0, ptr %br
  %bb = getelementptr i8, ptr %rect, i64 12
  store i32 %ch, ptr %bb
  %f2 = call i32 @FillRect(ptr %dc, ptr %rect, ptr %brush)
  br label %end
end:
  %del = call i32 @DeleteObject(ptr %brush)
  call void @LeaveCriticalSection(ptr %cs)
  ret void
}"#;

/// The procedure of the windows, on the thread of each: the kinds of events
/// (1 closing asked, 2 its size, 3 the pointer, 4 a button, 5 the wheel, 6
/// a key, 7 a character, 8 a part to draw again, 9 its dots per inch, 10
/// the focus); the last image painted again; `WM_APP` (the program closes
/// the window): destroyed, the thread ended; `WM_APP + 1`: a flush, its
/// signal (every message posted before it has been seen); every other
/// message to `DefWindowProcW`. A flush that finds characters in the queue
/// (`WM_CHAR` to `WM_UNICHAR`, posted by `TranslateMessage` for a key
/// seen before it, so behind it) is posted again behind them, 8 times at
/// most (D645: the character of a key posted just before a call of
/// `sysWinEvents` came at the next call, now and then).
const PROC: &str = r#"define internal i64 @isc.win.proc(ptr %hwnd, i32 %msg, i64 %wp, i64 %lp) {
entry:
  %peek = alloca [48 x i8], align 8
  %bi = call i64 @GetWindowLongPtrW(ptr %hwnd, i32 -21)
  %blk = inttoptr i64 %bi to ptr
  %none = icmp eq i64 %bi, 0
  br i1 %none, label %def, label %which
which:
  %lx16 = trunc i64 %lp to i16
  %x = sext i16 %lx16 to i64
  %lp16 = lshr i64 %lp, 16
  %ly16 = trunc i64 %lp16 to i16
  %y = sext i16 %ly16 to i64
  %lo = and i64 %lp, 65535
  %hi = and i64 %lp16, 65535
  switch i32 %msg, label %def [ i32 16, label %close
                                i32 5, label %size
                                i32 512, label %move
                                i32 513, label %ldown
                                i32 514, label %lup
                                i32 516, label %rdown
                                i32 517, label %rup
                                i32 519, label %mdown
                                i32 520, label %mup
                                i32 522, label %wheel
                                i32 256, label %kdown
                                i32 257, label %kup
                                i32 260, label %skdown
                                i32 261, label %skup
                                i32 258, label %char
                                i32 15, label %paint
                                i32 20, label %erase
                                i32 736, label %dpi
                                i32 7, label %focus
                                i32 8, label %blur
                                i32 32768, label %quit
                                i32 32769, label %flush
                                i32 32, label %setcursor
                                i32 32770, label %cursornow ]
close:
  call void @isc.win.push(ptr %blk, i64 1, i64 0, i64 0, i64 0)
  ret i64 0
size:
  call void @isc.win.push(ptr %blk, i64 2, i64 %lo, i64 %hi, i64 0)
  ret i64 0
move:
  call void @isc.win.push(ptr %blk, i64 3, i64 %x, i64 %y, i64 0)
  ret i64 0
ldown:
  br label %button
lup:
  br label %button
rdown:
  br label %button
rup:
  br label %button
mdown:
  br label %button
mup:
  br label %button
button:
  %btn = phi i64 [ 0, %ldown ], [ 0, %lup ], [ 1, %rdown ], [ 1, %rup ], [ 2, %mdown ], [ 2, %mup ]
  %down = phi i64 [ 1, %ldown ], [ 0, %lup ], [ 1, %rdown ], [ 0, %rup ], [ 1, %mdown ], [ 0, %mup ]
  call void @isc.win.push(ptr %blk, i64 3, i64 %x, i64 %y, i64 0)
  call void @isc.win.push(ptr %blk, i64 4, i64 %btn, i64 %down, i64 0)
  ret i64 0
wheel:
  %wp16 = lshr i64 %wp, 16
  %wd16 = trunc i64 %wp16 to i16
  %wd = sext i16 %wd16 to i64
  call void @isc.win.push(ptr %blk, i64 5, i64 %wd, i64 0, i64 0)
  ret i64 0
kdown:
  br label %key
kup:
  br label %key
key:
  %kd = phi i64 [ 1, %kdown ], [ 0, %kup ]
  %vk = and i64 %wp, 65535
  %scan = and i64 %hi, 511
  call void @isc.win.push(ptr %blk, i64 6, i64 %vk, i64 %kd, i64 %scan)
  ret i64 0
skdown:
  br label %skey
skup:
  br label %skey
skey:
  %skd = phi i64 [ 1, %skdown ], [ 0, %skup ]
  %svk = and i64 %wp, 65535
  %sscan = and i64 %hi, 511
  call void @isc.win.push(ptr %blk, i64 6, i64 %svk, i64 %skd, i64 %sscan)
  br label %def
char:
  %unit = and i64 %wp, 65535
  call void @isc.win.push(ptr %blk, i64 7, i64 %unit, i64 0, i64 0)
  ret i64 0
paint:
  %ps = alloca [72 x i8], align 8
  %pdc = call ptr @BeginPaint(ptr %hwnd, ptr %ps)
  call void @isc.win.repaint(ptr %blk, ptr %hwnd, ptr %pdc)
  %pend = call i32 @EndPaint(ptr %hwnd, ptr %ps)
  call void @isc.win.push(ptr %blk, i64 8, i64 0, i64 0, i64 0)
  ret i64 0
erase:
  ret i64 1
dpi:
  %d = and i64 %wp, 65535
  call void @isc.win.push(ptr %blk, i64 9, i64 %d, i64 0, i64 0)
  ; the size Windows proposes for the new density taken
  %r = inttoptr i64 %lp to ptr
  %l = load i32, ptr %r
  %pt = getelementptr i8, ptr %r, i64 4
  %t = load i32, ptr %pt
  %pr = getelementptr i8, ptr %r, i64 8
  %rr = load i32, ptr %pr
  %pb = getelementptr i8, ptr %r, i64 12
  %bb = load i32, ptr %pb
  %ww = sub i32 %rr, %l
  %wh = sub i32 %bb, %t
  %sp = call i32 @SetWindowPos(ptr %hwnd, ptr null, i32 %l, i32 %t, i32 %ww, i32 %wh, i32 20)
  ret i64 0
focus:
  call void @isc.win.push(ptr %blk, i64 10, i64 1, i64 0, i64 0)
  br label %def
blur:
  call void @isc.win.push(ptr %blk, i64 10, i64 0, i64 0, i64 0)
  br label %def
setcursor:
  ; (the inside of the window, HTCLIENT: the shape asked; elsewhere the
  ; system's, its borders)
  %ht = and i64 %lp, 65535
  %inside = icmp eq i64 %ht, 1
  br i1 %inside, label %shape, label %def
shape:
  %pcs = getelementptr i8, ptr %blk, i64 160
  %cid = load i64, ptr %pcs
  %cres = inttoptr i64 %cid to ptr
  %cur = call ptr @LoadCursorW(ptr null, ptr %cres)
  %prevc = call ptr @SetCursor(ptr %cur)
  ret i64 1
cursornow:
  ; the shape asked set at once when the pointer is over the window (it
  ; may not move)
  %gotpt = call i32 @GetCursorPos(ptr %peek)
  %ptv = load i64, ptr %peek
  %over = call ptr @WindowFromPoint(i64 %ptv)
  %ours = icmp eq ptr %over, %hwnd
  br i1 %ours, label %shape, label %done0
done0:
  ret i64 0
quit:
  %gone = call i64 @SetWindowLongPtrW(ptr %hwnd, i32 -21, i64 0)
  %destroyed = call i32 @DestroyWindow(ptr %hwnd)
  call void @PostQuitMessage(i32 0)
  ret i64 0
flush:
  %pending = call i32 @PeekMessageW(ptr %peek, ptr %hwnd, i32 258, i32 265, i32 0)
  %chars = icmp ne i32 %pending, 0
  %hops = icmp ult i64 %wp, 8
  %again = and i1 %chars, %hops
  br i1 %again, label %repost, label %signal
repost:
  %wp1 = add i64 %wp, 1
  %rp = call i32 @PostMessageW(ptr %hwnd, i32 32769, i64 %wp1, i64 0)
  ret i64 0
signal:
  %pfe = getelementptr i8, ptr %blk, i64 152
  %fe = load ptr, ptr %pfe
  %fs = call i32 @SetEvent(ptr %fe)
  ret i64 0
def:
  %r0 = call i64 @DefWindowProcW(ptr %hwnd, i32 %msg, i64 %wp, i64 %lp)
  ret i64 %r0
}"#;

/// `sysWinOpen(title, width, height) => [window, code]`: the class
/// registered (again: refused, nothing changed), the title in UTF-16, the
/// block of the window, its thread started, which makes a window whose
/// inside has `width` by `height` pixels, shown, then reads its messages;
/// the process aware of the density of each screen.
const OPEN: &str = r#"define internal { i64, i64 } @isc.win.open(ptr %p, i64 %n, i64 %w, i64 %h) {
entry:
  %wc = alloca [80 x i8], align 8
  %rect = alloca [16 x i8], align 8
  %aware = call i32 @SetProcessDpiAwarenessContext(ptr inttoptr (i64 -4 to ptr))
  %hinst = call ptr @GetModuleHandleW(ptr null)
  %cursor = call ptr @LoadCursorW(ptr null, ptr inttoptr (i64 32512 to ptr))
  store i32 80, ptr %wc
  %pstyle = getelementptr i8, ptr %wc, i64 4
  store i32 3, ptr %pstyle
  %pproc = getelementptr i8, ptr %wc, i64 8
  store ptr @isc.win.proc, ptr %pproc
  %pextra = getelementptr i8, ptr %wc, i64 16
  store i64 0, ptr %pextra
  %pinst = getelementptr i8, ptr %wc, i64 24
  store ptr %hinst, ptr %pinst
  %picon = getelementptr i8, ptr %wc, i64 32
  store ptr null, ptr %picon
  %pcur = getelementptr i8, ptr %wc, i64 40
  store ptr %cursor, ptr %pcur
  %pbg = getelementptr i8, ptr %wc, i64 48
  store ptr null, ptr %pbg
  %pmenu = getelementptr i8, ptr %wc, i64 56
  store ptr null, ptr %pmenu
  %pclass = getelementptr i8, ptr %wc, i64 64
  store ptr @isc.win.class, ptr %pclass
  %psm = getelementptr i8, ptr %wc, i64 72
  store ptr null, ptr %psm
  %atom = call i16 @RegisterClassExW(ptr %wc)
  ; the title in UTF-16, ended by a zero
  %n32 = trunc i64 %n to i32
  %units = call i32 @MultiByteToWideChar(i32 65001, i32 0, ptr %p, i32 %n32, ptr null, i32 0)
  %u64 = sext i32 %units to i64
  %tb = shl i64 %u64, 1
  %tb2 = add i64 %tb, 2
  %title = call ptr @isc_alloc(i64 %tb2)
  %done = call i32 @MultiByteToWideChar(i32 65001, i32 0, ptr %p, i32 %n32, ptr %title, i32 %units)
  %end = getelementptr i16, ptr %title, i64 %u64
  store i16 0, ptr %end
  ; the size of the window for an inside of w by h
  %w32 = trunc i64 %w to i32
  %h32 = trunc i64 %h to i32
  store i32 0, ptr %rect
  %rt = getelementptr i8, ptr %rect, i64 4
  store i32 0, ptr %rt
  %rr = getelementptr i8, ptr %rect, i64 8
  store i32 %w32, ptr %rr
  %rb = getelementptr i8, ptr %rect, i64 12
  store i32 %h32, ptr %rb
  %adj = call i32 @AdjustWindowRectEx(ptr %rect, i32 13565952, i32 0, i32 0)
  %l = load i32, ptr %rect
  %t = load i32, ptr %rt
  %r = load i32, ptr %rr
  %b = load i32, ptr %rb
  %ww = sub i32 %r, %l
  %wh = sub i32 %b, %t
  ; the block: the window (made by the thread), the queue, the lock, the
  ; signals, what the thread needs, no image yet
  %blk = call ptr @isc_alloc(i64 176)
  %queue = call ptr @isc_alloc(i64 131072)
  store ptr null, ptr %blk
  %pq = getelementptr i8, ptr %blk, i64 8
  store ptr %queue, ptr %pq
  %pn = getelementptr i8, ptr %blk, i64 16
  store i64 0, ptr %pn
  %pcap = getelementptr i8, ptr %blk, i64 24
  store i64 4096, ptr %pcap
  %cs = getelementptr i8, ptr %blk, i64 32
  call void @InitializeCriticalSection(ptr %cs)
  %avail = call ptr @CreateEventW(ptr null, i32 0, i32 0, ptr null)
  %pav = getelementptr i8, ptr %blk, i64 72
  store ptr %avail, ptr %pav
  %ready = call ptr @CreateEventW(ptr null, i32 0, i32 0, ptr null)
  %prd = getelementptr i8, ptr %blk, i64 88
  store ptr %ready, ptr %prd
  %pti = getelementptr i8, ptr %blk, i64 96
  store ptr %title, ptr %pti
  %pww = getelementptr i8, ptr %blk, i64 104
  store i32 %ww, ptr %pww
  %pwh = getelementptr i8, ptr %blk, i64 108
  store i32 %wh, ptr %pwh
  %pcode = getelementptr i8, ptr %blk, i64 112
  store i64 0, ptr %pcode
  %pimg = getelementptr i8, ptr %blk, i64 120
  store ptr null, ptr %pimg
  %piw = getelementptr i8, ptr %blk, i64 128
  store i64 0, ptr %piw
  %pih = getelementptr i8, ptr %blk, i64 136
  store i64 0, ptr %pih
  %pic = getelementptr i8, ptr %blk, i64 144
  store i64 0, ptr %pic
  %flushed = call ptr @CreateEventW(ptr null, i32 0, i32 0, ptr null)
  %pfl = getelementptr i8, ptr %blk, i64 152
  store ptr %flushed, ptr %pfl
  %pshape = getelementptr i8, ptr %blk, i64 160
  store i64 32512, ptr %pshape
  %th = call ptr @CreateThread(ptr null, i64 0, ptr @isc.win.thread, ptr %blk, i32 0, ptr null)
  %pth = getelementptr i8, ptr %blk, i64 80
  store ptr %th, ptr %pth
  %nothread = icmp eq ptr %th, null
  br i1 %nothread, label %nothr, label %wait
nothr:
  %te = call i32 @GetLastError()
  %te64 = zext i32 %te to i64
  %tcode = sub i64 0, %te64
  store i64 %tcode, ptr %pcode
  br label %check
wait:
  %wr = call i32 @WaitForSingleObject(ptr %ready, i32 -1)
  br label %check
check:
  %cr = call i32 @CloseHandle(ptr %ready)
  call void @isc_free(ptr %title)
  %hwnd = load ptr, ptr %blk
  %failed = icmp eq ptr %hwnd, null
  br i1 %failed, label %fail, label %made
fail:
  %code = load i64, ptr %pcode
  br i1 %nothread, label %clean, label %join
join:
  %jw = call i32 @WaitForSingleObject(ptr %th, i32 -1)
  %jc = call i32 @CloseHandle(ptr %th)
  br label %clean
clean:
  %ca = call i32 @CloseHandle(ptr %avail)
  %cf = call i32 @CloseHandle(ptr %flushed)
  call void @DeleteCriticalSection(ptr %cs)
  call void @isc_free(ptr %queue)
  call void @isc_free(ptr %blk)
  %r0 = insertvalue { i64, i64 } { i64 -1, i64 0 }, i64 %code, 1
  ret { i64, i64 } %r0
made:
  %bi = ptrtoint ptr %blk to i64
  %r1 = insertvalue { i64, i64 } { i64 0, i64 0 }, i64 %bi, 0
  ret { i64, i64 } %r1
}"#;

/// The thread of a window: the window made (its inside the size of the
/// block), its block given to its procedure, shown, the program told (or
/// the code of the failure); then its messages read and given to its
/// procedure until it is destroyed (`WM_QUIT`).
const THREAD: &str = r#"define internal i32 @isc.win.thread(ptr %blk) {
entry:
  %msg = alloca [48 x i8], align 8
  %hinst = call ptr @GetModuleHandleW(ptr null)
  %pti = getelementptr i8, ptr %blk, i64 96
  %title = load ptr, ptr %pti
  %pww = getelementptr i8, ptr %blk, i64 104
  %ww = load i32, ptr %pww
  %pwh = getelementptr i8, ptr %blk, i64 108
  %wh = load i32, ptr %pwh
  %prd = getelementptr i8, ptr %blk, i64 88
  %ready = load ptr, ptr %prd
  %hwnd = call ptr @CreateWindowExW(i32 0, ptr @isc.win.class, ptr %title, i32 281999360, i32 -2147483648, i32 -2147483648, i32 %ww, i32 %wh, ptr null, ptr null, ptr %hinst, ptr null)
  %failed = icmp eq ptr %hwnd, null
  br i1 %failed, label %fail, label %made
fail:
  %e = call i32 @GetLastError()
  %e64 = zext i32 %e to i64
  %code = sub i64 0, %e64
  %pcode = getelementptr i8, ptr %blk, i64 112
  store i64 %code, ptr %pcode
  %s0 = call i32 @SetEvent(ptr %ready)
  ret i32 0
made:
  %bi = ptrtoint ptr %blk to i64
  %old = call i64 @SetWindowLongPtrW(ptr %hwnd, i32 -21, i64 %bi)
  store ptr %hwnd, ptr %blk
  %shown = call i32 @ShowWindow(ptr %hwnd, i32 5)
  %upd = call i32 @UpdateWindow(ptr %hwnd)
  %s1 = call i32 @SetEvent(ptr %ready)
  br label %loop
loop:
  %got = call i32 @GetMessageW(ptr %msg, ptr null, i32 0, i32 0)
  %more = icmp sgt i32 %got, 0
  br i1 %more, label %give, label %out
give:
  %tr = call i32 @TranslateMessage(ptr %msg)
  %dr = call i64 @DispatchMessageW(ptr %msg)
  br label %loop
out:
  ret i32 0
}"#;

/// `sysWinEvents(window, ms) => [words, code]`: when its queue is empty,
/// the wait for the signal of an event, `ms` ms at most (below 0: no
/// limit); then a flush (`WM_APP + 1` posted, its signal awaited a second
/// at most: every message posted to the window before it has been given
/// to its procedure); the events, four words each, the queue emptied under
/// its lock.
const EVENTS: &str = r#"define internal { { ptr, i64 }, i64 } @isc.win.events(i64 %h, i64 %ms) {
entry:
  %blk = inttoptr i64 %h to ptr
  %cs = getelementptr i8, ptr %blk, i64 32
  %pn = getelementptr i8, ptr %blk, i64 16
  call void @EnterCriticalSection(ptr %cs)
  %n0 = load i64, ptr %pn
  call void @LeaveCriticalSection(ptr %cs)
  %empty = icmp eq i64 %n0, 0
  br i1 %empty, label %wait, label %flush
wait:
  %neg = icmp slt i64 %ms, 0
  %big = icmp sgt i64 %ms, 2147483647
  %t1 = select i1 %big, i64 2147483647, i64 %ms
  %t64 = select i1 %neg, i64 4294967295, i64 %t1
  %t = trunc i64 %t64 to i32
  %pav = getelementptr i8, ptr %blk, i64 72
  %avail = load ptr, ptr %pav
  %wr = call i32 @WaitForSingleObject(ptr %avail, i32 %t)
  br label %flush
flush:
  %hwnd = load ptr, ptr %blk
  %pfl = getelementptr i8, ptr %blk, i64 152
  %flushed = load ptr, ptr %pfl
  %posted = call i32 @PostMessageW(ptr %hwnd, i32 32769, i64 0, i64 0)
  %sent = icmp ne i32 %posted, 0
  br i1 %sent, label %await, label %take
await:
  %fw = call i32 @WaitForSingleObject(ptr %flushed, i32 1000)
  br label %take
take:
  call void @EnterCriticalSection(ptr %cs)
  %n = load i64, ptr %pn
  %words = shl i64 %n, 2
  %bytes = shl i64 %n, 5
  %zero = icmp eq i64 %n, 0
  %sz = select i1 %zero, i64 8, i64 %bytes
  %out = call ptr @isc_alloc(i64 %sz)
  %pq = getelementptr i8, ptr %blk, i64 8
  %q = load ptr, ptr %pq
  call void @llvm.memcpy.p0.p0.i64(ptr %out, ptr %q, i64 %bytes, i1 false)
  store i64 0, ptr %pn
  call void @LeaveCriticalSection(ptr %cs)
  %a0 = insertvalue { ptr, i64 } undef, ptr %out, 0
  %a1 = insertvalue { ptr, i64 } %a0, i64 %words, 1
  %v0 = insertvalue { { ptr, i64 }, i64 } poison, { ptr, i64 } %a1, 0
  %v1 = insertvalue { { ptr, i64 }, i64 } %v0, i64 0, 1
  ret { { ptr, i64 }, i64 } %v1
}"#;

/// `sysWinShow(window, pixels, width, height) => code`: the `width` by
/// `height` pixels (`0xAARRGGBB`, rows from the top: the order of the
/// bytes of Windows, blue first) drawn at the top left of the inside of the
/// window (`SetDIBitsToDevice`), and kept (a copy, under the lock) for the
/// procedure to paint again; -22 when there are fewer pixels.
const SHOW: &str = r#"define internal i64 @isc.win.show(i64 %h, ptr %px, i64 %n, i64 %w, i64 %hh) {
entry:
  %bmi = alloca [44 x i8], align 8
  %need = mul i64 %w, %hh
  %few = icmp slt i64 %n, %need
  %neg = icmp slt i64 %w, 1
  %negh = icmp slt i64 %hh, 1
  %b0 = or i1 %few, %neg
  %bad = or i1 %b0, %negh
  br i1 %bad, label %inval, label %draw
inval:
  ret i64 -22
draw:
  %blk = inttoptr i64 %h to ptr
  %hwnd = load ptr, ptr %blk
  ; the copy kept: its block grown when it is too small
  %cs = getelementptr i8, ptr %blk, i64 32
  call void @EnterCriticalSection(ptr %cs)
  %bytes = shl i64 %need, 2
  %pimg = getelementptr i8, ptr %blk, i64 120
  %img = load ptr, ptr %pimg
  %pic = getelementptr i8, ptr %blk, i64 144
  %cap = load i64, ptr %pic
  %small = icmp slt i64 %cap, %bytes
  br i1 %small, label %grow, label %keep
grow:
  %hasold = icmp ne ptr %img, null
  br i1 %hasold, label %freeold, label %alloc
freeold:
  call void @isc_free(ptr %img)
  br label %alloc
alloc:
  %nimg = call ptr @isc_alloc(i64 %bytes)
  store ptr %nimg, ptr %pimg
  store i64 %bytes, ptr %pic
  br label %copy
keep:
  br label %copy
copy:
  %dst = phi ptr [ %nimg, %alloc ], [ %img, %keep ]
  call void @llvm.memcpy.p0.p0.i64(ptr %dst, ptr %px, i64 %bytes, i1 false)
  %piw = getelementptr i8, ptr %blk, i64 128
  store i64 %w, ptr %piw
  %pih = getelementptr i8, ptr %blk, i64 136
  store i64 %hh, ptr %pih
  call void @LeaveCriticalSection(ptr %cs)
  %w32 = trunc i64 %w to i32
  %h32 = trunc i64 %hh to i32
  %top = sub i32 0, %h32
  store i32 40, ptr %bmi
  %pw = getelementptr i8, ptr %bmi, i64 4
  store i32 %w32, ptr %pw
  %ph = getelementptr i8, ptr %bmi, i64 8
  store i32 %top, ptr %ph
  %pplanes = getelementptr i8, ptr %bmi, i64 12
  store i16 1, ptr %pplanes
  %pbits = getelementptr i8, ptr %bmi, i64 14
  store i16 32, ptr %pbits
  %prest = getelementptr i8, ptr %bmi, i64 16
  store i64 0, ptr %prest
  %prest2 = getelementptr i8, ptr %bmi, i64 24
  store i64 0, ptr %prest2
  %prest3 = getelementptr i8, ptr %bmi, i64 32
  store i64 0, ptr %prest3
  %prest4 = getelementptr i8, ptr %bmi, i64 40
  store i32 0, ptr %prest4
  %dc = call ptr @GetDC(ptr %hwnd)
  %lines = call i32 @SetDIBitsToDevice(ptr %dc, i32 0, i32 0, i32 %w32, i32 %h32, i32 0, i32 0, i32 0, i32 %h32, ptr %px, ptr %bmi, i32 0)
  %rel = call i32 @ReleaseDC(ptr %hwnd, ptr %dc)
  %ok = icmp ne i32 %lines, 0
  br i1 %ok, label %fine, label %fail
fine:
  ret i64 0
fail:
  %e = call i32 @GetLastError()
  %e64 = zext i32 %e to i64
  %code = sub i64 0, %e64
  ret i64 %code
}"#;

/// `sysWinSize(window) => [width, height]`: of its inside.
const SIZE: &str = r#"define internal { i64, i64 } @isc.win.size(i64 %h) {
entry:
  %rect = alloca [16 x i8], align 8
  %blk = inttoptr i64 %h to ptr
  %hwnd = load ptr, ptr %blk
  store i64 0, ptr %rect
  %phalf = getelementptr i8, ptr %rect, i64 8
  store i64 0, ptr %phalf
  %ok = call i32 @GetClientRect(ptr %hwnd, ptr %rect)
  %pr = getelementptr i8, ptr %rect, i64 8
  %r = load i32, ptr %pr
  %pb = getelementptr i8, ptr %rect, i64 12
  %b = load i32, ptr %pb
  %w = sext i32 %r to i64
  %hh = sext i32 %b to i64
  %v0 = insertvalue { i64, i64 } poison, i64 %w, 0
  %v1 = insertvalue { i64, i64 } %v0, i64 %hh, 1
  ret { i64, i64 } %v1
}"#;

/// `sysWinPost(window, message, wparam, lparam) => code`: a message put in
/// the queue of the thread for the window (`PostMessageW`), as Windows does
/// for the input: a program wakes its loop with it, a test plays input.
const POST: &str = r#"define internal i64 @isc.win.post(i64 %h, i64 %m, i64 %wp, i64 %lp) {
entry:
  %blk = inttoptr i64 %h to ptr
  %hwnd = load ptr, ptr %blk
  %m32 = trunc i64 %m to i32
  %ok = call i32 @PostMessageW(ptr %hwnd, i32 %m32, i64 %wp, i64 %lp)
  %done = icmp ne i32 %ok, 0
  br i1 %done, label %fine, label %fail
fine:
  ret i64 0
fail:
  %e = call i32 @GetLastError()
  %e64 = zext i32 %e to i64
  %code = sub i64 0, %e64
  ret i64 %code
}"#;

/// `sysWinCapture(window) => [pixels, code]`: what the inside of the window
/// shows, `0xAARRGGBB` (the alpha as Windows gives it), rows from the top
/// (`BitBlt` into a bitmap of memory, `GetDIBits`).
const CAPTURE: &str = r#"define internal { { ptr, i64 }, i64 } @isc.win.capture(i64 %h) {
entry:
  %bmi = alloca [44 x i8], align 8
  %wh = call { i64, i64 } @isc.win.size(i64 %h)
  %w = extractvalue { i64, i64 } %wh, 0
  %hh = extractvalue { i64, i64 } %wh, 1
  %n = mul i64 %w, %hh
  %bytes0 = shl i64 %n, 2
  %zero = icmp slt i64 %n, 1
  %bytes = select i1 %zero, i64 4, i64 %bytes0
  %out = call ptr @isc_alloc(i64 %bytes)
  br i1 %zero, label %empty, label %grab
empty:
  %e0 = insertvalue { ptr, i64 } undef, ptr %out, 0
  %e1 = insertvalue { ptr, i64 } %e0, i64 0, 1
  %ev0 = insertvalue { { ptr, i64 }, i64 } poison, { ptr, i64 } %e1, 0
  %ev1 = insertvalue { { ptr, i64 }, i64 } %ev0, i64 0, 1
  ret { { ptr, i64 }, i64 } %ev1
grab:
  %blk = inttoptr i64 %h to ptr
  %hwnd = load ptr, ptr %blk
  %w32 = trunc i64 %w to i32
  %h32 = trunc i64 %hh to i32
  %top = sub i32 0, %h32
  %dc = call ptr @GetDC(ptr %hwnd)
  %mem = call ptr @CreateCompatibleDC(ptr %dc)
  %bmp = call ptr @CreateCompatibleBitmap(ptr %dc, i32 %w32, i32 %h32)
  %old = call ptr @SelectObject(ptr %mem, ptr %bmp)
  %copied = call i32 @BitBlt(ptr %mem, i32 0, i32 0, i32 %w32, i32 %h32, ptr %dc, i32 0, i32 0, i32 13369376)
  %back = call ptr @SelectObject(ptr %mem, ptr %old)
  store i32 40, ptr %bmi
  %pw = getelementptr i8, ptr %bmi, i64 4
  store i32 %w32, ptr %pw
  %ph = getelementptr i8, ptr %bmi, i64 8
  store i32 %top, ptr %ph
  %pplanes = getelementptr i8, ptr %bmi, i64 12
  store i16 1, ptr %pplanes
  %pbits = getelementptr i8, ptr %bmi, i64 14
  store i16 32, ptr %pbits
  %prest = getelementptr i8, ptr %bmi, i64 16
  store i64 0, ptr %prest
  %prest2 = getelementptr i8, ptr %bmi, i64 24
  store i64 0, ptr %prest2
  %prest3 = getelementptr i8, ptr %bmi, i64 32
  store i64 0, ptr %prest3
  %prest4 = getelementptr i8, ptr %bmi, i64 40
  store i32 0, ptr %prest4
  %lines = call i32 @GetDIBits(ptr %mem, ptr %bmp, i32 0, i32 %h32, ptr %out, ptr %bmi, i32 0)
  %d1 = call i32 @DeleteObject(ptr %bmp)
  %d2 = call i32 @DeleteDC(ptr %mem)
  %rel = call i32 @ReleaseDC(ptr %hwnd, ptr %dc)
  %ok = icmp ne i32 %lines, 0
  %e = call i32 @GetLastError()
  %e64 = zext i32 %e to i64
  %neg = sub i64 0, %e64
  %code = select i1 %ok, i64 0, i64 %neg
  %a0 = insertvalue { ptr, i64 } undef, ptr %out, 0
  %a1 = insertvalue { ptr, i64 } %a0, i64 %n, 1
  %v0 = insertvalue { { ptr, i64 }, i64 } poison, { ptr, i64 } %a1, 0
  %v1 = insertvalue { { ptr, i64 }, i64 } %v0, i64 %code, 1
  ret { { ptr, i64 }, i64 } %v1
}"#;

/// `sysWinClose(window) => code`: its thread asked to destroy the window
/// (`WM_APP`) and awaited, then its signals, its lock, its queue, its
/// image and its block given back; the handle of a failure (-1): nothing.
const CLOSE: &str = r#"define internal i64 @isc.win.close(i64 %h) {
entry:
  %none = icmp eq i64 %h, -1
  br i1 %none, label %nothing, label %some
nothing:
  ret i64 0
some:
  %blk = inttoptr i64 %h to ptr
  %hwnd = load ptr, ptr %blk
  %ok = call i32 @PostMessageW(ptr %hwnd, i32 32768, i64 0, i64 0)
  %posted = icmp ne i32 %ok, 0
  br i1 %posted, label %join, label %fail
join:
  %pth = getelementptr i8, ptr %blk, i64 80
  %th = load ptr, ptr %pth
  %jw = call i32 @WaitForSingleObject(ptr %th, i32 -1)
  %jc = call i32 @CloseHandle(ptr %th)
  %pav = getelementptr i8, ptr %blk, i64 72
  %avail = load ptr, ptr %pav
  %ca = call i32 @CloseHandle(ptr %avail)
  %pfl = getelementptr i8, ptr %blk, i64 152
  %flushed = load ptr, ptr %pfl
  %cf = call i32 @CloseHandle(ptr %flushed)
  %cs = getelementptr i8, ptr %blk, i64 32
  call void @DeleteCriticalSection(ptr %cs)
  %pq = getelementptr i8, ptr %blk, i64 8
  %q = load ptr, ptr %pq
  call void @isc_free(ptr %q)
  %pimg = getelementptr i8, ptr %blk, i64 120
  %img = load ptr, ptr %pimg
  %hasimg = icmp ne ptr %img, null
  br i1 %hasimg, label %freeimg, label %freeblk
freeimg:
  call void @isc_free(ptr %img)
  br label %freeblk
freeblk:
  call void @isc_free(ptr %blk)
  ret i64 0
fail:
  %e = call i32 @GetLastError()
  %e64 = zext i32 %e to i64
  %code = sub i64 0, %e64
  ret i64 %code
}"#;

/// The functions of Windows of the clipboard (D646).
const CLIP_DECLS: [&str; 13] = [
    "declare i32 @OpenClipboard(ptr)",
    "declare i32 @CloseClipboard()",
    "declare i32 @EmptyClipboard()",
    "declare ptr @SetClipboardData(i32, ptr)",
    "declare ptr @GetClipboardData(i32)",
    "declare ptr @GlobalAlloc(i32, i64)",
    "declare ptr @GlobalLock(ptr)",
    "declare i32 @GlobalUnlock(ptr)",
    "declare ptr @GlobalFree(ptr)",
    "declare i32 @lstrlenW(ptr)",
    "declare void @Sleep(i32)",
    "declare i32 @WideCharToMultiByte(i32, i32, ptr, i32, ptr, i32, ptr, ptr)",
    "declare i32 @MultiByteToWideChar(i32, i32, ptr, i32, ptr, i32)",
];

/// `sysWinCursor(window, shape) => code`: the shape of the pointer over the
/// inside of the window (0 an arrow, 1 a hand, 2 a bar of text: the cursors
/// 32512, 32649 and 32513 of the system) kept in its block, then set at once
/// by its thread (`WM_APP + 2`) when the pointer is over it; the procedure
/// sets it at each `WM_SETCURSOR` of its inside.
const CURSOR: &str = r#"define internal i64 @isc.win.cursor(i64 %h, i64 %shape) {
entry:
  %blk = inttoptr i64 %h to ptr
  %hand = icmp eq i64 %shape, 1
  %bar = icmp eq i64 %shape, 2
  %c1 = select i1 %hand, i64 32649, i64 32512
  %cid = select i1 %bar, i64 32513, i64 %c1
  %pcur = getelementptr i8, ptr %blk, i64 160
  store i64 %cid, ptr %pcur
  %hwnd = load ptr, ptr %blk
  %posted = call i32 @PostMessageW(ptr %hwnd, i32 32770, i64 0, i64 0)
  ret i64 0
}"#;

/// The clipboard opened for the window `hwnd`: tried 20 times 5 ms apart
/// (another program may hold it); true when opened.
const CLIP_OPEN: &str = r#"define internal i1 @isc.win.clipopen(ptr %hwnd) {
entry:
  br label %try
try:
  %k = phi i32 [ 0, %entry ], [ %k1, %again ]
  %ok = call i32 @OpenClipboard(ptr %hwnd)
  %opened = icmp ne i32 %ok, 0
  br i1 %opened, label %yes, label %more
more:
  %k1 = add i32 %k, 1
  %last = icmp sge i32 %k1, 20
  br i1 %last, label %no, label %again
again:
  call void @Sleep(i32 5)
  br label %try
yes:
  ret i1 true
no:
  ret i1 false
}"#;

/// `sysWinClipSet(window, text) => code`: the text put on the clipboard of
/// the system as UTF-16 (`CF_UNICODETEXT`, a block of `GlobalAlloc` the
/// system then owns), its owner the window.
const CLIP_SET: &str = r#"define internal i64 @isc.win.clipset(i64 %h, ptr %p, i64 %n) {
entry:
  %blk = inttoptr i64 %h to ptr
  %hwnd = load ptr, ptr %blk
  %opened = call i1 @isc.win.clipopen(ptr %hwnd)
  br i1 %opened, label %put, label %fail
put:
  %emptied = call i32 @EmptyClipboard()
  %n32 = trunc i64 %n to i32
  %units = call i32 @MultiByteToWideChar(i32 65001, i32 0, ptr %p, i32 %n32, ptr null, i32 0)
  %u64 = sext i32 %units to i64
  %tb = shl i64 %u64, 1
  %tb2 = add i64 %tb, 2
  %mem = call ptr @GlobalAlloc(i32 2, i64 %tb2)
  %nomem = icmp eq ptr %mem, null
  br i1 %nomem, label %closefail, label %fill
fill:
  %w = call ptr @GlobalLock(ptr %mem)
  %done = call i32 @MultiByteToWideChar(i32 65001, i32 0, ptr %p, i32 %n32, ptr %w, i32 %units)
  %end = getelementptr i16, ptr %w, i64 %u64
  store i16 0, ptr %end
  %unl = call i32 @GlobalUnlock(ptr %mem)
  %set = call ptr @SetClipboardData(i32 13, ptr %mem)
  %notset = icmp eq ptr %set, null
  br i1 %notset, label %freefail, label %closeok
freefail:
  %freed = call ptr @GlobalFree(ptr %mem)
  br label %closefail
closeok:
  %c1 = call i32 @CloseClipboard()
  ret i64 0
closefail:
  %e = call i32 @GetLastError()
  %c2 = call i32 @CloseClipboard()
  %e64 = zext i32 %e to i64
  %neg = sub i64 0, %e64
  %zero = icmp eq i64 %neg, 0
  %code = select i1 %zero, i64 -1, i64 %neg
  ret i64 %code
fail:
  %e2 = call i32 @GetLastError()
  %e264 = zext i32 %e2 to i64
  %neg2 = sub i64 0, %e264
  %zero2 = icmp eq i64 %neg2, 0
  %code2 = select i1 %zero2, i64 -1, i64 %neg2
  ret i64 %code2
}"#;

/// `sysWinClipGet(window) => [text, code]`: the text of the clipboard of
/// the system (`CF_UNICODETEXT`) in UTF-8; none there: an empty text.
const CLIP_GET: &str = r#"define internal { { ptr, i64 }, i64 } @isc.win.clipget(i64 %h) {
entry:
  %blk = inttoptr i64 %h to ptr
  %hwnd = load ptr, ptr %blk
  %opened = call i1 @isc.win.clipopen(ptr %hwnd)
  br i1 %opened, label %get, label %fail
get:
  %mem = call ptr @GetClipboardData(i32 13)
  %none = icmp eq ptr %mem, null
  br i1 %none, label %empty, label %read
read:
  %w = call ptr @GlobalLock(ptr %mem)
  %units = call i32 @lstrlenW(ptr %w)
  %bytes = call i32 @WideCharToMultiByte(i32 65001, i32 0, ptr %w, i32 %units, ptr null, i32 0, ptr null, ptr null)
  %b64 = sext i32 %bytes to i64
  %room = add i64 %b64, 1
  %out = call ptr @isc_alloc(i64 %room)
  %done = call i32 @WideCharToMultiByte(i32 65001, i32 0, ptr %w, i32 %units, ptr %out, i32 %bytes, ptr null, ptr null)
  %unl = call i32 @GlobalUnlock(ptr %mem)
  %c1 = call i32 @CloseClipboard()
  %d64 = sext i32 %done to i64
  %a0 = insertvalue { ptr, i64 } undef, ptr %out, 0
  %a1 = insertvalue { ptr, i64 } %a0, i64 %d64, 1
  %v0 = insertvalue { { ptr, i64 }, i64 } poison, { ptr, i64 } %a1, 0
  %v1 = insertvalue { { ptr, i64 }, i64 } %v0, i64 0, 1
  ret { { ptr, i64 }, i64 } %v1
empty:
  %c2 = call i32 @CloseClipboard()
  %eout = call ptr @isc_alloc(i64 1)
  %e0 = insertvalue { ptr, i64 } undef, ptr %eout, 0
  %e1 = insertvalue { ptr, i64 } %e0, i64 0, 1
  %ev0 = insertvalue { { ptr, i64 }, i64 } poison, { ptr, i64 } %e1, 0
  %ev1 = insertvalue { { ptr, i64 }, i64 } %ev0, i64 0, 1
  ret { { ptr, i64 }, i64 } %ev1
fail:
  %e = call i32 @GetLastError()
  %fout = call ptr @isc_alloc(i64 1)
  %e64 = zext i32 %e to i64
  %neg = sub i64 0, %e64
  %zero = icmp eq i64 %neg, 0
  %code = select i1 %zero, i64 -1, i64 %neg
  %f0 = insertvalue { ptr, i64 } undef, ptr %fout, 0
  %f1 = insertvalue { ptr, i64 } %f0, i64 0, 1
  %g0 = insertvalue { { ptr, i64 }, i64 } poison, { ptr, i64 } %f1, 0
  %g1 = insertvalue { { ptr, i64 }, i64 } %g0, i64 %code, 1
  ret { { ptr, i64 }, i64 } %g1
}"#;

/// The functions of Windows of `ALERT` and `PROGRAM`.
const ALERT_DECLS: [&str; 5] = [
    "declare i32 @MessageBoxW(ptr, ptr, ptr, i32)",
    "declare i32 @MultiByteToWideChar(i32, i32, ptr, i32, ptr, i32)",
    "declare i32 @WideCharToMultiByte(i32, i32, ptr, i32, ptr, i32, ptr, ptr)",
    "declare i32 @GetModuleFileNameW(ptr, ptr, i32)",
    "declare i32 @GetLastError()",
];

/// The text of UTF-8 `p`, `n` in UTF-16 ended by a zero, in a block of the
/// allocator (D643).
const WIDE: &str = r#"define internal ptr @isc.win.wide(ptr %p, i64 %n) {
entry:
  %n32 = trunc i64 %n to i32
  %units = call i32 @MultiByteToWideChar(i32 65001, i32 0, ptr %p, i32 %n32, ptr null, i32 0)
  %u64 = sext i32 %units to i64
  %tb = shl i64 %u64, 1
  %tb2 = add i64 %tb, 2
  %w = call ptr @isc_alloc(i64 %tb2)
  %done = call i32 @MultiByteToWideChar(i32 65001, i32 0, ptr %p, i32 %n32, ptr %w, i32 %units)
  %end = getelementptr i16, ptr %w, i64 %u64
  store i16 0, ptr %end
  ret ptr %w
}"#;

/// `sysWinAlert(title, text) => code`: `MessageBoxW` without an owner, its
/// button OK and the sign of a warning, brought in front
/// (`MB_SETFOREGROUND`); the button pressed (1), or the code of a failure.
const ALERT: &str = r#"define internal i64 @isc.win.alert(ptr %tp, i64 %tn, ptr %xp, i64 %xn) {
entry:
  %title = call ptr @isc.win.wide(ptr %tp, i64 %tn)
  %text = call ptr @isc.win.wide(ptr %xp, i64 %xn)
  %r = call i32 @MessageBoxW(ptr null, ptr %text, ptr %title, i32 65584)
  %e = call i32 @GetLastError()
  call void @isc_free(ptr %title)
  call void @isc_free(ptr %text)
  %ok = icmp ne i32 %r, 0
  %r64 = sext i32 %r to i64
  %e64 = zext i32 %e to i64
  %neg = sub i64 0, %e64
  %code = select i1 %ok, i64 %r64, i64 %neg
  ret i64 %code
}"#;

/// `sysWinProgram() => [path, code]`: `GetModuleFileNameW` of the program
/// (a path of 32767 units at most), in UTF-8 (`WideCharToMultiByte`).
const PROGRAM: &str = r#"define internal { { ptr, i64 }, i64 } @isc.win.program() {
entry:
  %wbuf = call ptr @isc_alloc(i64 65536)
  %n = call i32 @GetModuleFileNameW(ptr null, ptr %wbuf, i32 32768)
  %e = call i32 @GetLastError()
  %bad = icmp eq i32 %n, 0
  %full = icmp uge i32 %n, 32768
  %fail = or i1 %bad, %full
  br i1 %fail, label %failed, label %convert
convert:
  %bytes = call i32 @WideCharToMultiByte(i32 65001, i32 0, ptr %wbuf, i32 %n, ptr null, i32 0, ptr null, ptr null)
  %b64 = sext i32 %bytes to i64
  %room = add i64 %b64, 1
  %out = call ptr @isc_alloc(i64 %room)
  %done = call i32 @WideCharToMultiByte(i32 65001, i32 0, ptr %wbuf, i32 %n, ptr %out, i32 %bytes, ptr null, ptr null)
  call void @isc_free(ptr %wbuf)
  %d64 = sext i32 %done to i64
  %a0 = insertvalue { ptr, i64 } undef, ptr %out, 0
  %a1 = insertvalue { ptr, i64 } %a0, i64 %d64, 1
  %v0 = insertvalue { { ptr, i64 }, i64 } poison, { ptr, i64 } %a1, 0
  %v1 = insertvalue { { ptr, i64 }, i64 } %v0, i64 0, 1
  ret { { ptr, i64 }, i64 } %v1
failed:
  call void @isc_free(ptr %wbuf)
  %none = call ptr @isc_alloc(i64 1)
  %e64 = zext i32 %e to i64
  %neg0 = sub i64 0, %e64
  ; (a path too long: ERROR_INSUFFICIENT_BUFFER, 122)
  %neg = select i1 %full, i64 -122, i64 %neg0
  %f0 = insertvalue { ptr, i64 } undef, ptr %none, 0
  %f1 = insertvalue { ptr, i64 } %f0, i64 0, 1
  %g0 = insertvalue { { ptr, i64 }, i64 } poison, { ptr, i64 } %f1, 0
  %g1 = insertvalue { { ptr, i64 }, i64 } %g0, i64 %neg, 1
  ret { { ptr, i64 }, i64 } %g1
}"#;

impl Gen<'_> {
    /// The functions `defs` of `ALERT` and `PROGRAM` defined in the program,
    /// once each, and the functions of Windows they call (D643): none of
    /// the windows'.
    fn win_alert_defs(&mut self, defs: &[&str]) {
        for d in ALERT_DECLS.iter() {
            self.decls.insert(d.to_string());
        }
        for d in defs {
            let name = d.split('@').nth(1).and_then(|r| r.split('(').next()).unwrap_or(d);
            if self.decls.insert(format!("; the module win32: @{}", name)) {
                self.funcs.push(format!("{}\n", d));
            }
        }
    }

    /// The functions `defs` of the clipboard defined in the program, once
    /// each, and the functions of Windows they call (D646).
    fn clip_defs(&mut self, defs: &[&str]) {
        for d in CLIP_DECLS.iter().chain(ALERT_DECLS.iter()) {
            self.decls.insert(d.to_string());
        }
        for d in defs {
            let name = d.split('@').nth(1).and_then(|r| r.split('(').next()).unwrap_or(d);
            if self.decls.insert(format!("; the module win32: @{}", name)) {
                self.funcs.push(format!("{}\n", d));
            }
        }
    }

    /// The functions `defs` of this module defined in the program, once
    /// each, with those of the program (their blocks from its allocator, as
    /// `unix_defs`); the functions of Windows declared.
    fn win_defs(&mut self, defs: &[&str], capture: bool) {
        for d in DECLS.iter().chain(LOCK_DECLS.iter()).chain(if capture { CAPTURE_DECLS.iter() } else { [].iter() }) {
            self.decls.insert(d.to_string());
        }
        self.decls.insert(CLASS.to_string());
        for d in [PUSH, REPAINT, PROC, THREAD].iter().chain(defs) {
            let name = d.split('@').nth(1).and_then(|r| r.split('(').next()).unwrap_or(d);
            if self.decls.insert(format!("; the module win32: @{}", name)) {
                self.funcs.push(format!("{}\n", d));
            }
        }
    }

    /// A primitive of the module `win32` (D636), its arguments evaluated.
    pub(super) fn win32_op(&mut self, o: SysOp, vals: &[V]) -> V {
        match o {
            SysOp::WinOpen => {
                self.win_defs(&[OPEN], false);
                let (p, n) = self.str_parts(&vals[0]);
                v(self.ins(format!("call {{ i64, i64 }} @isc.win.open(ptr {}, i64 {}, i64 {}, i64 {})", p, n, vals[1].r, vals[2].r)), "{ i64, i64 }")
            }
            SysOp::WinEvents => {
                self.win_defs(&[EVENTS], false);
                let t = "{ { ptr, i64 }, i64 }";
                v(self.ins(format!("call {} @isc.win.events(i64 {}, i64 {})", t, vals[0].r, vals[1].r)), t)
            }
            SysOp::WinShow => {
                self.win_defs(&[SHOW], false);
                let (p, n) = self.str_parts(&vals[1]);
                v(self.ins(format!("call i64 @isc.win.show(i64 {}, ptr {}, i64 {}, i64 {}, i64 {})", vals[0].r, p, n, vals[2].r, vals[3].r)), "i64")
            }
            SysOp::WinSize => {
                self.win_defs(&[SIZE], false);
                v(self.ins(format!("call {{ i64, i64 }} @isc.win.size(i64 {})", vals[0].r)), "{ i64, i64 }")
            }
            SysOp::WinPost => {
                self.win_defs(&[POST], false);
                v(self.ins(format!("call i64 @isc.win.post(i64 {}, i64 {}, i64 {}, i64 {})", vals[0].r, vals[1].r, vals[2].r, vals[3].r)), "i64")
            }
            SysOp::WinCapture => {
                self.win_defs(&[SIZE, CAPTURE], true);
                let t = "{ { ptr, i64 }, i64 }";
                v(self.ins(format!("call {} @isc.win.capture(i64 {})", t, vals[0].r)), t)
            }
            SysOp::WinClose => {
                self.win_defs(&[CLOSE], false);
                v(self.ins(format!("call i64 @isc.win.close(i64 {})", vals[0].r)), "i64")
            }
            SysOp::WinAlert => {
                self.win_alert_defs(&[WIDE, ALERT]);
                let (tp, tn) = self.str_parts(&vals[0]);
                let (xp, xn) = self.str_parts(&vals[1]);
                v(self.ins(format!("call i64 @isc.win.alert(ptr {}, i64 {}, ptr {}, i64 {})", tp, tn, xp, xn)), "i64")
            }
            SysOp::WinCursor => {
                self.win_defs(&[CURSOR], false);
                v(self.ins(format!("call i64 @isc.win.cursor(i64 {}, i64 {})", vals[0].r, vals[1].r)), "i64")
            }
            SysOp::WinClipSet => {
                self.clip_defs(&[CLIP_OPEN, CLIP_SET]);
                let (p, n) = self.str_parts(&vals[1]);
                v(self.ins(format!("call i64 @isc.win.clipset(i64 {}, ptr {}, i64 {})", vals[0].r, p, n)), "i64")
            }
            SysOp::WinClipGet => {
                self.clip_defs(&[CLIP_OPEN, CLIP_GET]);
                let t = "{ { ptr, i64 }, i64 }";
                v(self.ins(format!("call {} @isc.win.clipget(i64 {})", t, vals[0].r)), t)
            }
            SysOp::WinProgram => {
                self.win_alert_defs(&[PROGRAM]);
                let t = "{ { ptr, i64 }, i64 }";
                v(self.ins(format!("call {} @isc.win.program()", t)), t)
            }
            _ => unreachable!("not a primitive of the module win32: {:?}", o),
        }
    }
}
