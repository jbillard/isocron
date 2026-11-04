//! The system under Linux on x86-64 (D287): the functions of the C library
//! that the runtime called, with the same names and meanings, by the system
//! calls themselves; no C library. An error is the negative code the kernel
//! gives (`-ENOENT`), not -1 and `errno`: there is no `errno` without a C
//! library (`fail_code` gives the code to the callers); `mmap` and `mremap`
//! give `MAP_FAILED` as before. The texts of the codes are those of glibc
//! (`isc_runtime_core::oserr`), the same in the interpreter.
//!
//! The only instruction that enters the kernel is in `syscall6`; every
//! function here is a call of it with the numbers of the system call.

use core::arch::{asm, global_asm};
use core::ffi::{c_char, c_int, c_void};

/// A system call with up to six arguments: its result, a negative code on
/// an error.
///
/// # Safety
/// The call must be one whose arguments are valid for it (pointers to
/// memory of the sizes it reads or writes): the kernel trusts them.
#[inline(always)]
pub unsafe fn syscall6(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize {
    let ret: isize;
    // SAFETY: the instruction of the system call of x86-64 Linux: its number
    // in rax, its arguments in rdi, rsi, rdx, r10, r8, r9; it clobbers rcx
    // and r11 and nothing else (the kernel saves the other registers); the
    // validity of the arguments is the caller's (see above)
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n as isize => ret,
            in("rdi") a,
            in("rsi") b,
            in("rdx") c,
            in("r10") d,
            in("r8") e,
            in("r9") f,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

// the numbers of the system calls of x86-64
const SYS_READ: usize = 0;
const SYS_WRITE: usize = 1;
const SYS_CLOSE: usize = 3;
const SYS_LSEEK: usize = 8;
const SYS_MMAP: usize = 9;
const SYS_MPROTECT: usize = 10;
const SYS_MUNMAP: usize = 11;
const SYS_RT_SIGACTION: usize = 13;
const SYS_PREAD64: usize = 17;
const SYS_MREMAP: usize = 25;
const SYS_MADVISE: usize = 28;
const SYS_NANOSLEEP: usize = 35;
const SYS_GETPID: usize = 39;
const SYS_FCNTL: usize = 72;
const SYS_GETTID: usize = 186;
const SYS_FUTEX: usize = 202;
const SYS_SCHED_GETAFFINITY: usize = 204;
const SYS_CLOCK_GETTIME: usize = 228;
const SYS_EXIT_GROUP: usize = 231;
const SYS_TGKILL: usize = 234;
const SYS_OPENAT: usize = 257;

const AT_FDCWD: isize = -100;
const SIGABRT: c_int = 6;
pub const MAP_FAILED: *mut c_void = !0usize as *mut c_void;

/// The code of a failed call (`r < 0`): the negative result of the kernel.
pub fn fail_code(r: isize) -> c_int {
    (-r) as c_int
}

/// # Safety
/// `path` is a string ending with a zero byte.
pub unsafe fn open(path: *const c_char, flags: c_int, mode: c_int) -> c_int {
    // SAFETY: openat reads the string `path` up to its zero byte
    unsafe { syscall6(SYS_OPENAT, AT_FDCWD as usize, path as usize, flags as usize, mode as usize, 0, 0) as c_int }
}

/// # Safety
/// `buf` has `n` bytes to write into.
pub unsafe fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize {
    // SAFETY: the kernel writes at most `n` bytes at `buf`
    unsafe { syscall6(SYS_READ, fd as usize, buf as usize, n, 0, 0, 0) }
}

/// # Safety
/// `buf` has `n` bytes to read.
pub unsafe fn write(fd: c_int, buf: *const c_void, n: usize) -> isize {
    // SAFETY: the kernel reads `n` bytes at `buf`
    unsafe { syscall6(SYS_WRITE, fd as usize, buf as usize, n, 0, 0, 0) }
}

/// # Safety
/// `buf` has `n` bytes to write into.
pub unsafe fn pread(fd: c_int, buf: *mut c_void, n: usize, off: i64) -> isize {
    // SAFETY: the kernel writes at most `n` bytes at `buf`
    unsafe { syscall6(SYS_PREAD64, fd as usize, buf as usize, n, off as usize, 0, 0) }
}

pub fn close(fd: c_int) -> c_int {
    // SAFETY: no memory is given to the kernel
    unsafe { syscall6(SYS_CLOSE, fd as usize, 0, 0, 0, 0, 0) as c_int }
}

pub fn lseek(fd: c_int, off: i64, whence: c_int) -> i64 {
    // SAFETY: no memory is given to the kernel
    unsafe { syscall6(SYS_LSEEK, fd as usize, off as usize, whence as usize, 0, 0, 0) as i64 }
}

/// `fcntl` with an integer argument (the commands the runtime gives).
pub fn fcntl(fd: c_int, cmd: c_int, arg: c_int) -> c_int {
    // SAFETY: the commands of the runtime (F_GETFL, F_SETLEASE) take an
    // integer, no memory
    unsafe { syscall6(SYS_FCNTL, fd as usize, cmd as usize, arg as usize, 0, 0, 0) as c_int }
}

/// `MAP_FAILED` on an error.
///
/// # Safety
/// With `MAP_FIXED`, `addr .. addr + len` is memory the caller may replace.
pub unsafe fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, off: i64) -> *mut c_void {
    // SAFETY: the mapping replaces only what the caller allows (see above)
    let r = unsafe { syscall6(SYS_MMAP, addr as usize, len, prot as usize, flags as usize, fd as usize, off as usize) };
    if (-4095..0).contains(&r) {
        MAP_FAILED
    } else {
        r as *mut c_void
    }
}

/// # Safety
/// Nothing of `addr .. addr + len` is used after.
pub unsafe fn munmap(addr: *mut c_void, len: usize) -> c_int {
    // SAFETY: the memory given back is the caller's to give (see above)
    unsafe { syscall6(SYS_MUNMAP, addr as usize, len, 0, 0, 0, 0) as c_int }
}

/// `MAP_FAILED` on an error.
///
/// # Safety
/// The mapping at `old` is the caller's to move; with `MREMAP_FIXED`,
/// `new .. new + new_len` is memory it may replace.
pub unsafe fn mremap(old: *mut c_void, old_len: usize, new_len: usize, flags: c_int, new: *mut c_void) -> *mut c_void {
    // SAFETY: see above
    let r = unsafe { syscall6(SYS_MREMAP, old as usize, old_len, new_len, flags as usize, new as usize, 0) };
    if (-4095..0).contains(&r) {
        MAP_FAILED
    } else {
        r as *mut c_void
    }
}

/// # Safety
/// For an advice that drops pages (`MADV_DONTNEED`), their contents are no
/// longer needed.
pub unsafe fn madvise(addr: *mut c_void, len: usize, advice: c_int) -> c_int {
    // SAFETY: see above; the other advices change no contents
    unsafe { syscall6(SYS_MADVISE, addr as usize, len, advice as usize, 0, 0, 0) as c_int }
}

/// # Safety
/// No access the new protection forbids is made after.
pub unsafe fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int {
    // SAFETY: see above
    unsafe { syscall6(SYS_MPROTECT, addr as usize, len, prot as usize, 0, 0, 0) as c_int }
}

/// # Safety
/// `req` is a valid `timespec` (seconds, nanoseconds); `rem` null or one.
pub unsafe fn nanosleep(req: *const [i64; 2], rem: *mut [i64; 2]) -> c_int {
    // SAFETY: the kernel reads `req` and writes `rem` (when not null)
    unsafe { syscall6(SYS_NANOSLEEP, req as usize, rem as usize, 0, 0, 0, 0) as c_int }
}

/// The bytes of the mask of the CPUs written (> 0), or a negative code.
///
/// # Safety
/// `mask` has `size` bytes.
pub unsafe fn sched_getaffinity(pid: c_int, size: usize, mask: *mut u8) -> c_int {
    // SAFETY: the kernel writes at most `size` bytes at `mask`
    unsafe { syscall6(SYS_SCHED_GETAFFINITY, pid as usize, size, mask as usize, 0, 0, 0) as c_int }
}

/// # Safety
/// `ts` is a valid `Timespec`.
pub unsafe fn clock_gettime(clk: c_int, ts: *mut super::Timespec) -> c_int {
    // SAFETY: the kernel writes one Timespec at `ts`
    unsafe { syscall6(SYS_CLOCK_GETTIME, clk as usize, ts as usize, 0, 0, 0, 0) as c_int }
}

/// A futex wait or wake (`FUTEX_WAIT_PRIVATE` with no timeout, or
/// `FUTEX_WAKE_PRIVATE`) on the word at `addr`.
pub fn futex(addr: &core::sync::atomic::AtomicU32, op: c_int, val: u32) -> isize {
    // SAFETY: the kernel reads (and waits on) the word of `addr`, a live
    // atomic; no timeout and no second word are given
    unsafe { syscall6(SYS_FUTEX, addr.as_ptr() as usize, op as usize, val as usize, 0, 0, 0) }
}

/// The text of glibc for a code.
pub fn strerror(code: c_int) -> *const c_char {
    isc_runtime_core::oserr::text_c(code).as_ptr() as *const c_char
}

/// # Safety
/// `s` is a string ending with a zero byte.
pub unsafe fn strlen(s: *const c_char) -> usize {
    let mut n = 0;
    // SAFETY: every byte up to the zero is part of the string (see above)
    while unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    n
}

/// The end of the process (every thread) with `code`.
pub fn exit(code: c_int) -> ! {
    loop {
        // SAFETY: exit_group gives no memory and does not return
        unsafe { syscall6(SYS_EXIT_GROUP, code as usize, 0, 0, 0, 0, 0) };
    }
}

/// The signal `sig` sent to this thread.
pub fn raise(sig: c_int) -> c_int {
    // SAFETY: getpid and gettid take nothing; tgkill gives no memory
    unsafe {
        let pid = syscall6(SYS_GETPID, 0, 0, 0, 0, 0, 0);
        let tid = syscall6(SYS_GETTID, 0, 0, 0, 0, 0, 0);
        syscall6(SYS_TGKILL, pid as usize, tid as usize, sig as usize, 0, 0, 0) as c_int
    }
}

/// The process stops on SIGABRT (as `abort` of C: the signal is not caught
/// by the runtime), else exits with the code of a death by it.
pub fn abort() -> ! {
    raise(SIGABRT);
    exit(128 + SIGABRT)
}

/// `struct sigaction` of the kernel of x86-64 (not glibc's: its mask is one
/// word, and a restorer is needed).
#[repr(C)]
pub struct SigAction {
    pub handler: usize,
    pub flags: u64,
    pub restorer: usize,
    pub mask: u64,
}

const SA_RESTORER: u64 = 0x0400_0000;

// The return of a handler of a signal: the kernel calls it with the stack
// of the frame of the signal, which rt_sigreturn (15) takes back.
global_asm!(".globl isc_sigreturn", ".type isc_sigreturn,@function", "isc_sigreturn:", "mov eax, 15", "syscall", "ud2");

/// The address of `isc_sigreturn`.
fn sigreturn_address() -> usize {
    let a: usize;
    // SAFETY: an address computed, nothing read or written
    unsafe { asm!("lea {}, [rip + isc_sigreturn]", out(reg) a, options(nomem, nostack, pure)) };
    a
}

/// The action of `sig` replaced by `act` (when not null), the one before
/// written into `old` (when not null).
///
/// # Safety
/// `act` and `old` are null or valid; the handler of `act` is a function of
/// the signature its flags say.
pub unsafe fn sigaction(sig: c_int, act: *const SigAction, old: *mut SigAction) -> c_int {
    let mut k;
    let given = if act.is_null() {
        core::ptr::null()
    } else {
        // SAFETY: `act` is valid (see above)
        k = unsafe { act.read() };
        k.flags |= SA_RESTORER;
        k.restorer = sigreturn_address();
        &k as *const SigAction
    };
    // SAFETY: the kernel reads `given` and writes `old` (each one action, the
    // size of the mask 8)
    unsafe { syscall6(SYS_RT_SIGACTION, sig as usize, given as usize, old as usize, 8, 0, 0) as c_int }
}
