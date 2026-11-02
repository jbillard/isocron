//! Calls of the C functions declared with `extern` (SPEC section 12,
//! DECISIONS D125), System V x86-64: the integer arguments (integers,
//! booleans, pointers, lengths) go in order to the 6 integer registers, the
//! floating-point ones to the 8 vector registers, each class on its own;
//! so one function type with 6 integers and 8 doubles calls any such
//! function (the registers it does not read are ignored), and the result
//! is read from `rax` or from `xmm0`. A `f32` travels in the low 32 bits of
//! its vector register. Variadic C functions are not supported (`al` is not
//! set).

use std::ffi::{c_char, c_int, c_void, CString};

extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlopen(file: *const c_char, flags: c_int) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

const RTLD_NOW: c_int = 2;
const RTLD_GLOBAL: c_int = 0x100;

/// An argument of a C call: in an integer or in a vector register.
#[derive(Clone, Copy, Debug)]
pub enum Arg {
    Int(i64),
    Float(f64),
}

/// Loads a shared library for the `extern` functions (`iscc run --link`):
/// `m` for `libm.so`, or a path.
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

type IntFn = unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, f64, f64, f64, f64, f64, f64, f64, f64) -> i64;
type FloatFn = unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, f64, f64, f64, f64, f64, f64, f64, f64) -> f64;

/// Calls the C function at `addr` (at most 6 integer and 8 floating-point
/// arguments, checked by the type checker); returns `rax` and `xmm0`.
///
/// # Safety
/// `addr` is a C function whose parameters are the `args`, in their
/// classes and in their order, and which is not variadic.
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
