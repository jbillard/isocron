//! LLD in the process (D248): when iscc is linked with the static
//! libraries of LLVM and LLD (`ISC_LLVM_STATIC`, `cfg(isc_lld)`), it links
//! programs itself, without any linker installed.

/// LLD is in this iscc.
pub fn available() -> bool {
    cfg!(isc_lld)
}

#[cfg(isc_lld)]
extern "C" {
    fn isc_lld_link(argc: i32, argv: *const *const std::ffi::c_char) -> i32;
}

/// One link at a time: LLD keeps state from one to the next.
#[cfg(isc_lld)]
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs LLD with `args` (`args[0]` gives the flavor, as the name of the
/// program: `ld.lld` for ELF and MinGW, with `-m i386pep`, `wasm-ld`); its
/// messages on the standard error.
pub fn link(args: &[std::ffi::OsString]) -> Result<(), String> {
    #[cfg(isc_lld)]
    {
        use std::ffi::CString;
        let owned: Vec<CString> = args
            .iter()
            .map(|a| CString::new(a.to_string_lossy().as_bytes()).map_err(|_| format!("a NUL in the argument {:?} of the linker", a)))
            .collect::<Result<_, _>>()?;
        let ptrs: Vec<*const std::ffi::c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let code = unsafe { isc_lld_link(ptrs.len() as i32, ptrs.as_ptr()) };
        if code == 0 {
            Ok(())
        } else {
            Err(format!("the linker (LLD) failed ({})", code))
        }
    }
    #[cfg(not(isc_lld))]
    {
        let _ = args;
        Err("this iscc has no linker of its own (built without ISC_LLVM_STATIC)".into())
    }
}
