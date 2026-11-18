//! Isocron code generator: IR to LLVM IR (`emit`), then machine code
//! through the LLVM C API (`backend`).

pub mod backend;
pub mod lld;
pub mod emit;
pub mod split;

pub use emit::{emit, emit_pre, emit_using, emit_with, pre_declarations, Isa, Options, PreManifest};

/// The time of a phase of the compiler since `since`, written to the error
/// output when `ISC_TIME` is set (where the time of a build goes: the
/// front end, the generated IR, the passes of LLVM, its code generator, the
/// link).
pub fn phase_time(label: &str, since: std::time::Instant) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("ISC_TIME").is_some()) {
        eprintln!("iscc time: {:<28} {:8.2} s", label, since.elapsed().as_secs_f64());
    }
}
