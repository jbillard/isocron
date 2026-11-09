//! The static programs for Linux without a C library (D289): their
//! relocations, checked at their build as their start reads them (D545).

mod common;
use common::*;

/// A minimal ELF file of 64 bits whose only segment is a dynamic one with
/// the entries `dynamic` (tag, value), ended by DT_NULL.
fn elf_with_dynamic(dynamic: &[(u64, u64)]) -> Vec<u8> {
    let mut e = vec![0u8; 64 + 56];
    e[..4].copy_from_slice(b"\x7fELF");
    e[4] = 2;
    e[0x20..0x28].copy_from_slice(&64u64.to_le_bytes());
    e[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
    e[0x38..0x3a].copy_from_slice(&1u16.to_le_bytes());
    let at = e.len() as u64;
    let h = 64;
    e[h..h + 4].copy_from_slice(&2u32.to_le_bytes());
    e[h + 8..h + 16].copy_from_slice(&at.to_le_bytes());
    e[h + 32..h + 40].copy_from_slice(&(16 * (dynamic.len() as u64 + 1)).to_le_bytes());
    for &(tag, val) in dynamic.iter().chain([(0, 0)].iter()) {
        e.extend_from_slice(&tag.to_le_bytes());
        e.extend_from_slice(&val.to_le_bytes());
    }
    e
}

/// A static program holds only `R_X86_64_RELATIVE` relocations, the kind its
/// start (`isc_start`) applies; another kind, a table of another kind
/// (`DT_RELR`, `DT_REL`, `DT_JMPREL`) or a library needed would be skipped
/// by it without a word: the check of its build names them.
#[test]
fn static_programs_have_only_relative_relocations() {
    let src = "action main(args: string[k]): i32 {\n    print([\"a\", \"bc\"]);\n    print(sum(map(iota(1000), (i) => i)));\n    return 0;\n}\n";
    let bin = compile(src, "static_relocations");
    let r = run_bin(&bin, &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, "[a, bc]\n499500\n"), "{}", r.stderr);
    let mut elf = std::fs::read(&bin).unwrap();
    let (n, at) = iscc::check_static_relocations(&elf).unwrap();
    assert!(n > 0, "no relocation in a static program");
    // the first one made `R_X86_64_64` (1)
    elf[at + 8..at + 16].copy_from_slice(&1u64.to_le_bytes());
    let e = iscc::check_static_relocations(&elf).unwrap_err();
    assert!(e.contains("relocation of type 1"), "{}", e);
    for (tag, what) in [(36, "dynamic tag 36"), (17, "dynamic tag 17"), (23, "dynamic tag 23"), (1, "DT_NEEDED")] {
        let e = iscc::check_static_relocations(&elf_with_dynamic(&[(tag, 0)])).unwrap_err();
        assert!(e.contains(what), "{}: {}", tag, e);
    }
    assert_eq!(iscc::check_static_relocations(&elf_with_dynamic(&[])), Ok((0, 0)));
    assert!(iscc::check_static_relocations(b"not an ELF file").is_err());
}
