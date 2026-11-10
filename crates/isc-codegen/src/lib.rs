//! Isocron code generator: IR to LLVM IR (`emit`), then machine code
//! through the LLVM C API (`backend`).

pub mod backend;
pub mod lld;
pub mod emit;

pub use emit::{emit, emit_pre, emit_using, emit_with, pre_declarations, Isa, Options, PreManifest};
