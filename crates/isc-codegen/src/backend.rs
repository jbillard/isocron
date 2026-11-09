//! LLVM back-end through the LLVM C API of the system `libLLVM-21.so`.
//!
//! The textual IR is parsed and verified, optimized with the `default<O3>`
//! pipeline for the host CPU (the equivalent of `-O3 -march=native`), and
//! emitted as an object file (and optionally as assembly).

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;

type Ref = *mut c_void;

extern "C" {
    fn LLVMContextCreate() -> Ref;
    fn LLVMContextDispose(c: Ref);
    fn LLVMCreateMemoryBufferWithMemoryRangeCopy(data: *const c_char, len: usize, name: *const c_char) -> Ref;
    fn LLVMParseIRInContext(ctx: Ref, buf: Ref, out: *mut Ref, msg: *mut *mut c_char) -> i32;
    fn LLVMVerifyModule(m: Ref, action: i32, msg: *mut *mut c_char) -> i32;
    fn LLVMDisposeMessage(msg: *mut c_char);
    fn LLVMDisposeModule(m: Ref);
    fn LLVMCloneModule(m: Ref) -> Ref;
    fn LLVMInitializeX86TargetInfo();
    fn LLVMInitializeX86Target();
    fn LLVMInitializeX86TargetMC();
    fn LLVMInitializeX86AsmPrinter();
    fn LLVMInitializeX86AsmParser();
    fn LLVMInitializeWebAssemblyTargetInfo();
    fn LLVMInitializeWebAssemblyTarget();
    fn LLVMInitializeWebAssemblyTargetMC();
    fn LLVMInitializeWebAssemblyAsmPrinter();
    fn LLVMInitializeWebAssemblyAsmParser();
    fn LLVMGetDefaultTargetTriple() -> *mut c_char;
    fn LLVMParseCommandLineOptions(argc: i32, argv: *const *const c_char, overview: *const c_char);
    fn LLVMGetTargetFromTriple(triple: *const c_char, t: *mut Ref, msg: *mut *mut c_char) -> i32;
    fn LLVMGetHostCPUName() -> *mut c_char;
    fn LLVMGetHostCPUFeatures() -> *mut c_char;
    fn LLVMCreateTargetMachine(
        t: Ref,
        triple: *const c_char,
        cpu: *const c_char,
        features: *const c_char,
        level: i32,
        reloc: i32,
        model: i32,
    ) -> Ref;
    fn LLVMDisposeTargetMachine(tm: Ref);
    fn LLVMCreateTargetDataLayout(tm: Ref) -> Ref;
    fn LLVMSetModuleDataLayout(m: Ref, dl: Ref);
    fn LLVMDisposeTargetData(dl: Ref);
    fn LLVMSetTarget(m: Ref, triple: *const c_char);
    fn LLVMCreatePassBuilderOptions() -> Ref;
    fn LLVMDisposePassBuilderOptions(o: Ref);
    fn LLVMRunPasses(m: Ref, passes: *const c_char, tm: Ref, o: Ref) -> Ref;
    fn LLVMGetErrorMessage(e: Ref) -> *mut c_char;
    fn LLVMDisposeErrorMessage(m: *mut c_char);
    fn LLVMTargetMachineEmitToFile(tm: Ref, m: Ref, file: *const c_char, kind: i32, msg: *mut *mut c_char) -> i32;
}

const RETURN_STATUS: i32 = 2;
const OPT_AGGRESSIVE: i32 = 3;
const RELOC_STATIC: i32 = 1;
const RELOC_PIC: i32 = 2;

/// LLVM triple of `iscc build --target wasm32-wasi` (WASI preview 1).
pub const WASM_TRIPLE: &str = "wasm32-unknown-wasi";
/// WebAssembly features used: 128-bit SIMD, saturating float-to-int
/// conversions, `memory.copy`/`memory.fill`, sign extension. All are part of
/// WebAssembly 2.0 and supported by the usual engines.
pub const WASM_FEATURES: &str = "+simd128,+nontrapping-fptoint,+bulk-memory,+sign-ext,+mutable-globals";
const MODEL_DEFAULT: i32 = 0;
const ASSEMBLY: i32 = 0;
const OBJECT: i32 = 1;

unsafe fn take_msg(m: *mut c_char) -> String {
    if m.is_null() {
        return String::new();
    }
    let s = CStr::from_ptr(m).to_string_lossy().to_string();
    LLVMDisposeMessage(m);
    s
}

fn cstr(s: &str) -> CString {
    CString::new(s).expect("nul byte")
}

/// The host CPU name, as LLVM sees it (`-march=native`).
pub fn host_cpu() -> String {
    unsafe { take_msg(LLVMGetHostCPUName()) }
}

/// Compiles textual LLVM IR to an object file; optionally writes assembly.
pub fn compile(ir: &str, obj: &Path, asm: Option<&Path>) -> Result<(), String> {
    compile_with(ir, obj, asm, crate::emit::Isa::Native)
}

/// The host CPU features, as LLVM sees them (`+avx2,-avx512f,...`).
pub fn host_features() -> String {
    unsafe { take_msg(LLVMGetHostCPUFeatures()) }
}

/// The host CPU features, without any AVX-512 feature for `Isa::Avx2`,
/// nor the extensions Valgrind does not decode (GFNI, VAES, VPCLMULQDQ:
/// LLVM uses GFNI for the masks of vector code, D140); `Isa::Avx2` is the
/// target of the runs under Valgrind.
pub fn features_for(host: &str, isa: crate::emit::Isa) -> String {
    if isa != crate::emit::Isa::Avx2 {
        return host.to_string();
    }
    host.split(',')
        .map(|f| {
            let name = f.trim_start_matches(['+', '-']);
            if name.starts_with("avx512")
                || name.starts_with("avx10")
                || name == "evex512"
                || name == "gfni"
                || name == "vaes"
                || name == "vpclmulqdq"
            {
                format!("-{}", name)
            } else {
                f.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Registers the targets once per process. LLVM's registration is not safe
/// to run from two threads at once: each could link a target into the list
/// of targets, which then loops, and `lookupTarget` never returns (the tests
/// compile from several threads; D85).
fn init_llvm() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| unsafe {
        LLVMInitializeX86TargetInfo();
        LLVMInitializeX86Target();
        LLVMInitializeX86TargetMC();
        LLVMInitializeX86AsmPrinter();
        LLVMInitializeX86AsmParser();
        LLVMInitializeWebAssemblyTargetInfo();
        LLVMInitializeWebAssemblyTarget();
        LLVMInitializeWebAssemblyTargetMC();
        LLVMInitializeWebAssemblyAsmPrinter();
        LLVMInitializeWebAssemblyAsmParser();
        // experiments only: options of LLVM's code generator, for example
        // `ISC_LLVM_ARGS="-align-loops=64"` (once per process)
        if let Ok(a) = std::env::var("ISC_LLVM_ARGS") {
            let mut owned = vec![cstr("iscc")];
            owned.extend(a.split_whitespace().map(cstr));
            let argv: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
            let ov = cstr("");
            LLVMParseCommandLineOptions(argv.len() as i32, argv.as_ptr(), ov.as_ptr());
        }
    });
}

pub fn compile_with(ir: &str, obj: &Path, asm: Option<&Path>, isa: crate::emit::Isa) -> Result<(), String> {
    compile_for(ir, obj, asm, isa, false, false)
}

/// The triple of the Windows target (D246).
pub const WINDOWS_TRIPLE: &str = "x86_64-pc-windows-gnu";

/// The triple of the Windows target with Microsoft's ABI (D542).
pub const WINDOWS_MSVC_TRIPLE: &str = "x86_64-pc-windows-msvc";

/// `compile_with` for Windows too (`windows`: the object of
/// `x86_64-pc-windows-gnu`, for the CPU that `isa` names).
pub fn compile_for(ir: &str, obj: &Path, asm: Option<&Path>, isa: crate::emit::Isa, windows: bool, msvc: bool) -> Result<(), String> {
    init_llvm();
    unsafe {
        let wasm = isa.is_wasm();
        let ctx = LLVMContextCreate();
        let name = cstr("isocron");
        let buf = LLVMCreateMemoryBufferWithMemoryRangeCopy(ir.as_ptr() as *const c_char, ir.len(), name.as_ptr());
        let mut m: Ref = std::ptr::null_mut();
        let mut msg: *mut c_char = std::ptr::null_mut();
        if LLVMParseIRInContext(ctx, buf, &mut m, &mut msg) != 0 {
            let e = take_msg(msg);
            LLVMContextDispose(ctx);
            return Err(format!("LLVM could not parse the generated IR: {}", e));
        }
        if LLVMVerifyModule(m, RETURN_STATUS, &mut msg) != 0 {
            let e = take_msg(msg);
            LLVMDisposeModule(m);
            LLVMContextDispose(ctx);
            return Err(format!("the generated IR is invalid: {}", e));
        }
        take_msg(msg);
        let triple = if wasm {
            cstr(WASM_TRIPLE)
        } else if windows && msvc {
            cstr(WINDOWS_MSVC_TRIPLE)
        } else if windows {
            cstr(WINDOWS_TRIPLE)
        } else {
            cstr(&take_msg(LLVMGetDefaultTargetTriple()))
        };
        let mut target: Ref = std::ptr::null_mut();
        if LLVMGetTargetFromTriple(triple.as_ptr(), &mut target, &mut msg) != 0 {
            return Err(format!("unknown target: {}", take_msg(msg)));
        }
        let host_features = take_msg(LLVMGetHostCPUFeatures());
        let (cpu, features) = if wasm {
            let f = if isa == crate::emit::Isa::Wasm32Threads { format!("{},+atomics", WASM_FEATURES) } else { WASM_FEATURES.into() };
            (cstr("generic"), cstr(&f))
        } else if isa == crate::emit::Isa::Sse {
            (cstr("goldmont"), cstr(""))
        } else if isa == crate::emit::Isa::Avx {
            (cstr("ivybridge"), cstr(""))
        } else {
            (cstr(&take_msg(LLVMGetHostCPUName())), cstr(&features_for(&host_features, isa)))
        };
        let reloc = if wasm { RELOC_STATIC } else { RELOC_PIC };
        let tm = LLVMCreateTargetMachine(target, triple.as_ptr(), cpu.as_ptr(), features.as_ptr(), OPT_AGGRESSIVE, reloc, MODEL_DEFAULT);
        LLVMSetTarget(m, triple.as_ptr());
        let dl = LLVMCreateTargetDataLayout(tm);
        LLVMSetModuleDataLayout(m, dl);
        let opts = LLVMCreatePassBuilderOptions();
        let passes = cstr("default<O3>");
        let err = LLVMRunPasses(m, passes.as_ptr(), tm, opts);
        LLVMDisposePassBuilderOptions(opts);
        if !err.is_null() {
            let em = LLVMGetErrorMessage(err);
            let s = CStr::from_ptr(em).to_string_lossy().to_string();
            LLVMDisposeErrorMessage(em);
            return Err(format!("LLVM optimization failed: {}", s));
        }
        let mut result = Ok(());
        if let Some(a) = asm {
            let clone = LLVMCloneModule(m);
            let f = cstr(&a.to_string_lossy());
            if LLVMTargetMachineEmitToFile(tm, clone, f.as_ptr(), ASSEMBLY, &mut msg) != 0 {
                result = Err(format!("cannot emit assembly: {}", take_msg(msg)));
            }
            LLVMDisposeModule(clone);
        }
        if result.is_ok() {
            let f = cstr(&obj.to_string_lossy());
            if LLVMTargetMachineEmitToFile(tm, m, f.as_ptr(), OBJECT, &mut msg) != 0 {
                result = Err(format!("cannot emit the object file: {}", take_msg(msg)));
            }
        }
        LLVMDisposeTargetData(dl);
        LLVMDisposeModule(m);
        LLVMDisposeTargetMachine(tm);
        LLVMContextDispose(ctx);
        result
    }
}
