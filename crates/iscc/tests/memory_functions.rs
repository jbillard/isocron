//! The functions of memory of the runtime (`isc-mem`, D288) in programs:
//! copies (concatenations, slices, `repeat`) and comparisons of strings of
//! every length up to 300 and of some larger ones (past the caches among
//! them), at places of every alignment, compiled for each instruction set
//! (each its variant of the runtime) and run on threads: the same result
//! as the interpreter. A program is static, without a C library (D289):
//! no dynamic loader, nothing imported.

mod common;
use common::*;

const PROBE: &str = r#"function probe(text: string, m: i64, at: i64): i64 {
    const a = text.slice(at, at + m);
    const b = text.slice(at + 36, at + 36 + m);
    const k = m / 2;
    const c = m > 0 ? b.slice(0, k) + "!" + b.slice(k + 1, m) : b;
    const d = a + c;
    const e = d.slice(m, m + m);
    return (a == b ? 1 : 0) + (a < c ? 2 : 0) + (c < a ? 4 : 0) + (e == c ? 8 : 0) + (d.length == m + m ? 16 : 0) + (a.repeat(2).slice(m, m + m) == a ? 32 : 0);
}

action main(args: string[k]): i32 {
    const text = "abcdefghijklmnopqrstuvwxyz0123456789".repeat(300000);
    print(map(iota(300), (m) => probe(text, m, m % 37)));
    print(map([1000, 2047, 2048, 2112, 4097, 65536, 1048583, 9437261], (m) => probe(text, m, m % 37)));
    return 0;
}
"#;

#[test]
fn copies_and_comparisons_of_every_length() {
    // every probe finds its strings as built: equal (1), the changed one
    // below (4), copied (8, 16, 32); for 0 bytes, equal both ways
    let line = |ms: &mut dyn Iterator<Item = usize>| format!("[{}]", ms.map(|m| if m == 0 { "57" } else { "61" }).collect::<Vec<_>>().join(", "));
    let want = format!("{}\n{}\n", line(&mut (0..300)), line(&mut (1..9)));
    assert_eq!(interp_ok(PROBE, &[]), want, "interpreter");
    let mut isas = vec![isc_codegen::Isa::Sse];
    if std::arch::is_x86_feature_detected!("avx2") {
        isas.push(isc_codegen::Isa::Avx2);
    }
    if std::arch::is_x86_feature_detected!("avx512bw") && std::arch::is_x86_feature_detected!("avx512vl") {
        isas.push(isc_codegen::Isa::Avx512);
    }
    for isa in isas {
        let name = format!("memfn_{:?}", isa);
        let bin = compile_with(PROBE, &name, isc_codegen::Options { isa, ..isc_codegen::Options::default() });
        let r = run_bin_threads(&bin, &[], Some(4));
        assert_eq!(r.code, 0, "{:?}: {}", isa, r.stderr);
        assert_eq!(r.stdout, want, "{:?}", isa);
        // the functions of memory its own, never imported; a static program
        // (D289: no dynamic loader, nothing imported) unless built to run
        // with glibc's allocator (the second run of the suite)
        let elf = std::fs::read(&bin).unwrap();
        let imported = imports(&elf);
        for f in ["memcpy", "memmove", "memset", "memcmp", "bcmp", "strlen"] {
            assert!(!imported.iter().any(|n| n == f), "{:?} imports {}", isa, f);
        }
        if std::env::var("ISC_ALLOC").as_deref() != Ok("glibc") {
            assert!(!has_interpreter(&elf), "{:?}: a dynamic loader", isa);
            assert_eq!(imported, Vec::<String>::new(), "{:?}", isa);
        }
    }
}

/// The names of the symbols an ELF64 executable imports: those of its
/// dynamic symbol table (`.dynsym`, its strings in the section it links
/// to) that are not defined (section index 0).
fn imports(elf: &[u8]) -> Vec<String> {
    let u16_at = |o: usize| u16::from_le_bytes(elf[o..o + 2].try_into().unwrap()) as usize;
    let u32_at = |o: usize| u32::from_le_bytes(elf[o..o + 4].try_into().unwrap()) as usize;
    let u64_at = |o: usize| u64::from_le_bytes(elf[o..o + 8].try_into().unwrap()) as usize;
    let (shoff, shentsize, shnum) = (u64_at(0x28), u16_at(0x3a), u16_at(0x3c));
    let section = |k: usize| shoff + k * shentsize;
    let mut names = Vec::new();
    for k in 0..shnum {
        let h = section(k);
        // SHT_DYNSYM
        if u32_at(h + 4) != 11 {
            continue;
        }
        let (off, size, link, entsize) = (u64_at(h + 0x18), u64_at(h + 0x20), u32_at(h + 0x28), u64_at(h + 0x38));
        let strtab = u64_at(section(link) + 0x18);
        for e in (off..off + size).step_by(entsize) {
            let (name, shndx) = (u32_at(e), u16_at(e + 6));
            if name == 0 || shndx != 0 {
                continue;
            }
            let start = strtab + name;
            let end = start + elf[start..].iter().position(|&c| c == 0).unwrap();
            names.push(String::from_utf8_lossy(&elf[start..end]).into_owned());
        }
    }
    names
}

/// Whether an ELF64 executable names a dynamic loader (`PT_INTERP`).
fn has_interpreter(elf: &[u8]) -> bool {
    let u16_at = |o: usize| u16::from_le_bytes(elf[o..o + 2].try_into().unwrap()) as usize;
    let u32_at = |o: usize| u32::from_le_bytes(elf[o..o + 4].try_into().unwrap());
    let phoff = u64::from_le_bytes(elf[0x20..0x28].try_into().unwrap()) as usize;
    (0..u16_at(0x38)).any(|k| u32_at(phoff + k * u16_at(0x36)) == 3)
}
