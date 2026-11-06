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
const SYS_FSTAT: usize = 5;
const SYS_LSEEK: usize = 8;
const SYS_MMAP: usize = 9;
const SYS_MPROTECT: usize = 10;
const SYS_MUNMAP: usize = 11;
const SYS_RT_SIGACTION: usize = 13;
const SYS_IOCTL: usize = 16;
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

/// The type and the permissions of the file of `fd` (`st_mode` of `fstat`),
/// or None.
pub fn file_mode(fd: c_int) -> Option<u32> {
    // `struct stat` of x86-64: 144 bytes, `st_mode` at byte 24
    let mut st = [0u64; 18];
    // SAFETY: `fstat` writes the 144 bytes of `st`
    let r = unsafe { syscall6(SYS_FSTAT, fd as usize, st.as_mut_ptr() as usize, 0, 0, 0, 0) };
    (r >= 0).then_some(st[3] as u32)
}

/// `fd` is a terminal (`ioctl(TCGETS)` succeeds, as `isatty`).
pub fn is_terminal(fd: c_int) -> bool {
    const TCGETS: usize = 0x5401;
    // `struct termios`: 60 bytes
    let mut t = [0u32; 16];
    // SAFETY: `TCGETS` writes at most the 60 bytes of a `struct termios`
    unsafe { syscall6(SYS_IOCTL, fd as usize, TCGETS, t.as_mut_ptr() as usize, 0, 0, 0) == 0 }
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

/// The `clock_gettime` of the vDSO of the process (D360), or `NO_VDSO`, or
/// 0 before the first reading of a clock.
static VDSO_CLOCK: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
const NO_VDSO: usize = 1;

/// The time of a clock: by the `clock_gettime` of the vDSO (D360), the code
/// the kernel maps into every process to read its clocks without entering
/// it (vdso(7), as the runtime of Go does without a C library: 149.7 ns a
/// `time.now()` by the system call, 17.7 for the floor by the vDSO); by the
/// system call when the process has no vDSO (`vdso=0`, Valgrind) or it has
/// not that function.
///
/// # Safety
/// `ts` is a valid `Timespec`.
pub unsafe fn clock_gettime(clk: c_int, ts: *mut super::Timespec) -> c_int {
    use core::sync::atomic::Ordering::Relaxed;
    let mut f = VDSO_CLOCK.load(Relaxed);
    if f == 0 {
        // SAFETY: the environment noted by `isc_args`, the first call of a
        // program, is the one the kernel made (see `vdso_symbol`)
        f = unsafe { vdso_symbol(b"__vdso_clock_gettime") }.unwrap_or(NO_VDSO);
        VDSO_CLOCK.store(f, Relaxed);
    }
    if f != NO_VDSO {
        // SAFETY: a function of the vDSO of this process, of the type of
        // `clock_gettime` (vdso(7): `__vdso_clock_gettime`, LINUX_2.6); it
        // writes one Timespec at `ts`
        unsafe {
            let g: unsafe extern "C" fn(c_int, *mut super::Timespec) -> c_int = core::mem::transmute(f);
            return g(clk, ts);
        }
    }
    // SAFETY: the kernel writes one Timespec at `ts`
    unsafe { syscall6(SYS_CLOCK_GETTIME, clk as usize, ts as usize, 0, 0, 0, 0) as c_int }
}

/// The address of the function `name` of the vDSO, from the auxiliary
/// vector the kernel puts after the environment on the first stack
/// (`AT_SYSINFO_EHDR`, the ELF image of the vDSO), its program headers
/// (the first `PT_LOAD` gives where it was linked, `PT_DYNAMIC` its tables)
/// and its dynamic symbols (counted by `DT_HASH`, or `DT_GNU_HASH`); none
/// when an image or a table is missing.
///
/// # Safety
/// The environment noted by `isc_args` is that of the first stack of the
/// process (`argv[argc + 1 ..]`), and the image the kernel mapped is whole.
unsafe fn vdso_symbol(name: &[u8]) -> Option<usize> {
    const AT_SYSINFO_EHDR: usize = 33;
    const PT_LOAD: u32 = 1;
    const PT_DYNAMIC: u32 = 2;
    const DT_HASH: i64 = 4;
    const DT_STRTAB: i64 = 5;
    const DT_SYMTAB: i64 = 6;
    const DT_GNU_HASH: i64 = 0x6fff_fef5;
    const STT_FUNC: u8 = 2;
    unsafe fn rd<T: Copy>(p: usize) -> T {
        unsafe { core::ptr::read_unaligned(p as *const T) }
    }
    // SAFETY: the words of the first stack after the environment (its null,
    // then the pairs of the auxiliary vector until AT_NULL), then the image
    // of the vDSO, read within its headers and tables (see above)
    unsafe {
        let mut e = super::ENVIRON.load(core::sync::atomic::Ordering::Relaxed) as *const usize;
        if e.is_null() {
            return None;
        }
        while *e != 0 {
            e = e.add(1);
        }
        let mut a = e.add(1);
        let mut base = 0usize;
        while *a != 0 {
            if *a == AT_SYSINFO_EHDR {
                base = *a.add(1);
            }
            a = a.add(2);
        }
        if base == 0 || rd::<[u8; 4]>(base) != *b"\x7fELF" || rd::<u8>(base + 4) != 2 {
            return None;
        }
        let (phoff, phentsize, phnum) = (rd::<u64>(base + 0x20) as usize, rd::<u16>(base + 0x36) as usize, rd::<u16>(base + 0x38) as usize);
        let (mut load, mut dynamic) = (None, 0usize);
        for i in 0..phnum {
            let ph = base + phoff + i * phentsize;
            match rd::<u32>(ph) {
                PT_LOAD if load.is_none() => load = Some(base.wrapping_add(rd::<u64>(ph + 8) as usize).wrapping_sub(rd::<u64>(ph + 16) as usize)),
                PT_DYNAMIC => dynamic = base + rd::<u64>(ph + 8) as usize,
                _ => {}
            }
        }
        let load = load?;
        if dynamic == 0 {
            return None;
        }
        let (mut strtab, mut symtab, mut hash, mut gnu) = (0usize, 0usize, 0usize, 0usize);
        let mut d = dynamic;
        loop {
            let tag = rd::<i64>(d);
            let at = load.wrapping_add(rd::<u64>(d + 8) as usize);
            match tag {
                0 => break,
                DT_HASH => hash = at,
                DT_STRTAB => strtab = at,
                DT_SYMTAB => symtab = at,
                DT_GNU_HASH => gnu = at,
                _ => {}
            }
            d += 16;
        }
        if strtab == 0 || symtab == 0 {
            return None;
        }
        let count = if hash != 0 {
            // nbucket, nchain: one entry of the chains a symbol
            rd::<u32>(hash + 4) as usize
        } else if gnu != 0 {
            // the highest symbol of the buckets, then its chain to its end
            let (nbuckets, symoffset, bloom) = (rd::<u32>(gnu) as usize, rd::<u32>(gnu + 4) as usize, rd::<u32>(gnu + 8) as usize);
            let buckets = gnu + 16 + 8 * bloom;
            let chains = buckets + 4 * nbuckets;
            let mut last = (0..nbuckets).map(|i| rd::<u32>(buckets + 4 * i) as usize).max().unwrap_or(0);
            if last < symoffset {
                symoffset
            } else {
                while rd::<u32>(chains + 4 * (last - symoffset)) & 1 == 0 {
                    last += 1;
                }
                last + 1
            }
        } else {
            return None;
        };
        for i in 0..count {
            let sym = symtab + 24 * i;
            let (st_name, info, shndx, value) = (rd::<u32>(sym) as usize, rd::<u8>(sym + 4), rd::<u16>(sym + 6), rd::<u64>(sym + 8) as usize);
            if shndx == 0 || info & 0xf != STT_FUNC {
                continue;
            }
            let s = strtab + st_name;
            if (0..name.len()).all(|j| rd::<u8>(s + j) == name[j]) && rd::<u8>(s + name.len()) == 0 {
                return Some(load.wrapping_add(value));
            }
        }
        None
    }
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

// the calls of the primitives of the system of the standard library (D322)
const SYS_GETCWD: usize = 79;
const SYS_GETDENTS64: usize = 217;
const SYS_MKDIRAT: usize = 258;
const SYS_NEWFSTATAT: usize = 262;
const SYS_UNLINKAT: usize = 263;
const SYS_RENAMEAT: usize = 264;

/// `newfstatat(AT_FDCWD, path, st, 0)`: the kind (1 a file, 2 a folder, 3
/// something else), the size, the time of the last change in nanoseconds,
/// or the negative code of the kernel.
///
/// # Safety
/// `path` is a string ending with a zero byte.
pub unsafe fn stat(path: *const c_char) -> Result<(i64, i64, i64), isize> {
    // `struct stat` of x86-64: 144 bytes; the mode at 24, the size at 48,
    // the time of the last change at 88 (seconds) and 96 (nanoseconds)
    let mut st = [0u64; 18];
    // SAFETY: the kernel reads `path` up to its zero byte and writes 144
    // bytes at `st`
    let r = unsafe { syscall6(SYS_NEWFSTATAT, AT_FDCWD as usize, path as usize, st.as_mut_ptr() as usize, 0, 0, 0) };
    if r < 0 {
        return Err(r);
    }
    let mode = (st[3] & 0xFFFF_FFFF) as u32;
    let kind = match mode & 0o170000 {
        0o100000 => 1,
        0o040000 => 2,
        _ => 3,
    };
    Ok((kind, st[6] as i64, (st[11] as i64).wrapping_mul(1_000_000_000).wrapping_add(st[12] as i64)))
}

/// `getdents64(fd, buf, n)`: the bytes of the entries written, or the
/// negative code of the kernel.
///
/// # Safety
/// `buf` has `n` bytes to write into.
pub unsafe fn getdents(fd: c_int, buf: *mut u8, n: usize) -> isize {
    // SAFETY: the kernel writes at most `n` bytes at `buf`
    unsafe { syscall6(SYS_GETDENTS64, fd as usize, buf as usize, n, 0, 0, 0) }
}

/// `mkdirat(AT_FDCWD, path, 0o777)`.
///
/// # Safety
/// `path` is a string ending with a zero byte.
pub unsafe fn mkdir(path: *const c_char) -> isize {
    // SAFETY: the kernel reads `path` up to its zero byte
    unsafe { syscall6(SYS_MKDIRAT, AT_FDCWD as usize, path as usize, 0o777, 0, 0, 0) }
}

/// `unlinkat(AT_FDCWD, path, flags)`: a file, or an empty folder with
/// `AT_REMOVEDIR` (0x200).
///
/// # Safety
/// `path` is a string ending with a zero byte.
pub unsafe fn unlink(path: *const c_char, folder: bool) -> isize {
    // SAFETY: the kernel reads `path` up to its zero byte
    unsafe { syscall6(SYS_UNLINKAT, AT_FDCWD as usize, path as usize, if folder { 0x200 } else { 0 }, 0, 0, 0) }
}

/// `renameat(AT_FDCWD, from, AT_FDCWD, to)`.
///
/// # Safety
/// `from` and `to` are strings ending with a zero byte.
pub unsafe fn rename(from: *const c_char, to: *const c_char) -> isize {
    // SAFETY: the kernel reads both strings up to their zero bytes
    unsafe { syscall6(SYS_RENAMEAT, AT_FDCWD as usize, from as usize, AT_FDCWD as usize, to as usize, 0, 0) }
}

/// `getcwd(buf, n)`: the bytes written with the zero byte, or the negative
/// code of the kernel.
///
/// # Safety
/// `buf` has `n` bytes to write into.
pub unsafe fn getcwd(buf: *mut u8, n: usize) -> isize {
    // SAFETY: the kernel writes at most `n` bytes at `buf`
    unsafe { syscall6(SYS_GETCWD, buf as usize, n, 0, 0, 0, 0) }
}
