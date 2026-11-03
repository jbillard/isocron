//! Calls of the C functions declared with `extern` (SPEC section 12,
//! DECISIONS D125), System V x86-64: the integer arguments (integers,
//! booleans, pointers, lengths) go in order to the 6 integer registers, the
//! floating-point ones to the 8 vector registers, each class on its own;
//! so one function type with 6 integers and 8 doubles calls any such
//! function (the registers it does not read are ignored), and the result
//! is read from `rax` or from `xmm0`. A `f32` travels in the low 32 bits of
//! its vector register. Variadic C functions are not supported (`al` is not
//! set).

//!
//! Under Windows (D247), Win64: the position of an argument gives its
//! register (the first four in `rcx`, `rdx`, `r8`, `r9` or `xmm0` to
//! `xmm3`, the others on the stack); each of the first four is put in both
//! (as the caller of a variadic function does), so one call by a few lines
//! of assembly serves any function; the libraries are DLLs (`LoadLibraryW`,
//! `GetProcAddress`).

use std::ffi::CString;
#[cfg(not(windows))]
use std::ffi::{c_char, c_int, c_void};

#[cfg(not(windows))]
extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlopen(file: *const c_char, flags: c_int) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

#[cfg(not(windows))]
const RTLD_NOW: c_int = 2;
#[cfg(not(windows))]
const RTLD_GLOBAL: c_int = 0x100;

/// An argument of a C call: in an integer or in a vector register.
#[derive(Clone, Copy, Debug)]
pub enum Arg {
    Int(i64),
    Float(f64),
}

/// Loads a shared library for the `extern` functions (`iscc run --link`):
/// `m` for `libm.so`, or a path.
#[cfg(not(windows))]
pub fn load_library(lib: &str) -> Result<(), String> {
    let path = if lib.contains('/') || lib.ends_with(".so") || lib.contains(".so.") { lib.to_string() } else { format!("lib{}.so", lib) };
    let c = CString::new(path.clone()).map_err(|_| format!("invalid library name `{}`", lib))?;
    let h = unsafe { dlopen(c.as_ptr(), RTLD_NOW | RTLD_GLOBAL) };
    if h.is_null() {
        let why = unsafe { std::ffi::CStr::from_ptr(dlerror()) }.to_string_lossy().to_string();
        return Err(format!("cannot load the library `{}`: {}", path, why));
    }
    Ok(())
}

/// The address of a C function, in the libraries of the process (libc and
/// libm loaded if needed) and those of `load_library`.
#[cfg(not(windows))]
pub fn lookup(name: &str) -> Option<usize> {
    let c = CString::new(name).ok()?;
    let find = || unsafe { dlsym(std::ptr::null_mut(), c.as_ptr()) } as usize;
    let mut a = find();
    if a == 0 {
        for lib in ["libm.so.6", "libc.so.6"] {
            let _ = load_library(lib);
        }
        a = find();
    }
    (a != 0).then_some(a)
}

#[cfg(not(windows))]
type IntFn = unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, f64, f64, f64, f64, f64, f64, f64, f64) -> i64;
#[cfg(not(windows))]
type FloatFn = unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, f64, f64, f64, f64, f64, f64, f64, f64) -> f64;

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        pub fn LoadLibraryW(name: *const u16) -> *mut c_void;
        pub fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        pub fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        pub fn GetLastError() -> u32;
    }
    /// The modules searched by `lookup`, as addresses.
    pub static MODULES: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
}

/// Loads a DLL for the `extern` functions (`iscc run --link`): `name` for
/// `name.dll`, or a path.
#[cfg(windows)]
pub fn load_library(lib: &str) -> Result<(), String> {
    let path = if lib.contains('/') || lib.contains('\\') || lib.to_ascii_lowercase().ends_with(".dll") { lib.to_string() } else { format!("{}.dll", lib) };
    let w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let h = unsafe { win::LoadLibraryW(w.as_ptr()) };
    if h.is_null() {
        return Err(format!("cannot load the library `{}`: error {}", path, unsafe { win::GetLastError() }));
    }
    win::MODULES.lock().unwrap().push(h as usize);
    Ok(())
}

/// The address of a C function: in the program, the C libraries of
/// Windows (`ucrtbase`, `msvcrt`), `kernel32` (the functions a program
/// for Windows is linked with) and the DLLs of `load_library`.
#[cfg(windows)]
pub fn lookup(name: &str) -> Option<usize> {
    let c = CString::new(name).ok()?;
    {
        let mut m = win::MODULES.lock().unwrap();
        if m.is_empty() {
            m.push(unsafe { win::GetModuleHandleW(std::ptr::null()) } as usize);
            for lib in ["ucrtbase.dll", "msvcrt.dll", "kernel32.dll"] {
                let w: Vec<u16> = lib.encode_utf16().chain(std::iter::once(0)).collect();
                let h = unsafe { win::LoadLibraryW(w.as_ptr()) };
                if !h.is_null() {
                    m.push(h as usize);
                }
            }
        }
    }
    let m = win::MODULES.lock().unwrap();
    m.iter().map(|&h| unsafe { win::GetProcAddress(h as *mut std::ffi::c_void, c.as_ptr() as *const u8) } as usize).find(|&a| a != 0)
}

/// Calls the C function at `addr` (Win64: at most 14 arguments, the type
/// checker allows 6 integers and 8 floats); returns `rax` and `xmm0`.
///
/// # Safety
/// As the other `call`.
#[cfg(windows)]
pub unsafe fn call(addr: usize, args: &[Arg], _float_result: bool) -> (i64, f64) {
    let mut slots = [0u64; 14];
    for (i, a) in args.iter().enumerate() {
        slots[i] = match *a {
            Arg::Int(v) => v as u64,
            Arg::Float(v) => v.to_bits(),
        };
    }
    let rax: i64;
    let xmm0: f64;
    // the stack: 32 bytes of shadow space, then the slots 4 .. 14, aligned
    // on 16 bytes at the call
    std::arch::asm!(
        "mov r12, rsp",
        "and rsp, -16",
        "sub rsp, 112",
        "mov rax, [r10 + 32]", "mov [rsp + 32], rax",
        "mov rax, [r10 + 40]", "mov [rsp + 40], rax",
        "mov rax, [r10 + 48]", "mov [rsp + 48], rax",
        "mov rax, [r10 + 56]", "mov [rsp + 56], rax",
        "mov rax, [r10 + 64]", "mov [rsp + 64], rax",
        "mov rax, [r10 + 72]", "mov [rsp + 72], rax",
        "mov rax, [r10 + 80]", "mov [rsp + 80], rax",
        "mov rax, [r10 + 88]", "mov [rsp + 88], rax",
        "mov rax, [r10 + 96]", "mov [rsp + 96], rax",
        "mov rax, [r10 + 104]", "mov [rsp + 104], rax",
        "mov rcx, [r10]",
        "mov rdx, [r10 + 8]",
        "mov r8, [r10 + 16]",
        "mov r9, [r10 + 24]",
        "movq xmm0, rcx",
        "movq xmm1, rdx",
        "movq xmm2, r8",
        "movq xmm3, r9",
        "call r11",
        "mov rsp, r12",
        in("r10") slots.as_ptr(),
        in("r11") addr,
        out("r12") _,
        out("rax") rax,
        out("xmm0") xmm0,
        clobber_abi("win64"),
    );
    (rax, xmm0)
}

/// Calls the C function at `addr` (at most 6 integer and 8 floating-point
/// arguments, checked by the type checker); returns `rax` and `xmm0`.
///
/// # Safety
/// `addr` is a C function whose parameters are the `args`, in their
/// classes and in their order, and which is not variadic.
#[cfg(not(windows))]
pub unsafe fn call(addr: usize, args: &[Arg], float_result: bool) -> (i64, f64) {
    let mut g = [0i64; 6];
    let mut f = [0f64; 8];
    let (mut ng, mut nf) = (0, 0);
    for a in args {
        match *a {
            Arg::Int(v) => {
                g[ng] = v;
                ng += 1;
            }
            Arg::Float(v) => {
                f[nf] = v;
                nf += 1;
            }
        }
    }
    if float_result {
        let fp: FloatFn = std::mem::transmute(addr);
        (0, fp(g[0], g[1], g[2], g[3], g[4], g[5], f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7]))
    } else {
        let fp: IntFn = std::mem::transmute(addr);
        (fp(g[0], g[1], g[2], g[3], g[4], g[5], f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7]), 0.0)
    }
}
