//! The DLLs of the system that only some programs need (`ws2_32`,
//! `secur32`, `crypt32`, `ncrypt`), loaded at the first call of one of
//! their functions (D541): a program that does not use the network imports
//! only `kernel32`, and Windows loads nothing else at its start. A DLL is
//! loaded from the directory of the system alone
//! (`LOAD_LIBRARY_SEARCH_SYSTEM32`), never from the directory of the
//! program or the current one.

use core::ffi::c_void;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;

/// The address of the function `name` (ASCII, ended by a zero byte) of the
/// DLL `dll` of the system (ASCII): the DLL loaded if it is not yet (Windows
/// counts the loads of a DLL; it stays as long as the process). A DLL of the
/// system or a function of it that is missing is a fault of the system: a
/// panic.
pub fn resolve(dll: &str, name: &str) -> usize {
    let mut w = [0u16; 64];
    assert!(dll.len() < w.len() && dll.is_ascii(), "the name of a DLL of the system");
    for (i, b) in dll.bytes().enumerate() {
        w[i] = b as u16;
    }
    assert!(name.ends_with('\0'));
    // SAFETY: a name of UTF-16 ended by a zero, no file handle, a flag of
    // the documentation; the module of the DLL and a name ended by a zero
    let f = unsafe {
        let m = LoadLibraryExW(w.as_ptr(), core::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32);
        if m.is_null() {
            panic!("cannot load {} from the directory of the system", dll);
        }
        GetProcAddress(m, name.as_ptr())
    };
    if f.is_null() {
        panic!("{} has no function {}", dll, name.trim_end_matches('\0'));
    }
    f as usize
}

/// Functions of a DLL of the system declared as in an `extern "system"`
/// block, each loaded at its first call (`resolve`), its address kept for
/// the next ones: `unsafe fn`s of the same names and signatures.
#[macro_export]
macro_rules! system_dll {
    ($dll:literal; $( fn $name:ident ( $( $arg:ident : $ty:ty ),* $(,)? ) $( -> $ret:ty )? ; )*) => {
        $(
            #[allow(non_snake_case, clippy::too_many_arguments)]
            #[inline]
            unsafe fn $name( $( $arg : $ty ),* ) $( -> $ret )? {
                static AT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
                let mut f = AT.load(core::sync::atomic::Ordering::Relaxed);
                if f == 0 {
                    // two threads may both resolve it: the same address
                    f = $crate::winload::resolve($dll, concat!(stringify!($name), "\0"));
                    AT.store(f, core::sync::atomic::Ordering::Relaxed);
                }
                // SAFETY: the address of the function of that name in the
                // DLL, whose signature is the one declared (the caller's
                // contract, as for an `extern` block)
                let f: unsafe extern "system" fn( $( $ty ),* ) $( -> $ret )? = unsafe { core::mem::transmute::<usize, _>(f) };
                unsafe { f( $( $arg ),* ) }
            }
        )*
    };
}
