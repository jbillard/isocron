# Third-party software in the binaries of Isocron

The released `iscc` (Linux) and `iscc.exe` (Windows) contain the software
below; the programs that `iscc` builds contain the runtime of Isocron (MIT,
see `LICENSE`), which contains the parts marked *in programs*. The full
texts of the licenses are in `licenses/`.

| Component | Version | License | Where |
| :-- | :-- | :-- | :-- |
| LLVM and LLD | 21.1.8 | Apache-2.0 WITH LLVM-exception (`licenses/LLVM-LICENSE.TXT`) | linked into iscc |
| The Rust standard library (`core`, `alloc`, `std`, `compiler_builtins`) | the Rust of the build | MIT OR Apache-2.0 | iscc; `core`, `alloc` and `compiler_builtins` *in programs* |
| regex-automata, regex-syntax | 0.4.18, 0.8.11 | MIT OR Apache-2.0 | iscc, *in programs* |
| memchr, aho-corasick | 2.8.3, 1.1.5 | Unlicense OR MIT | iscc, *in programs* |
| num-bigint, num-integer, num-traits | 0.4.8, 0.1.47, 0.2.19 | MIT OR Apache-2.0 | iscc, *in programs* |
| libm | 0.2.16 | MIT | iscc, *in programs* |
| MinGW-w64 (start files, `libmingw32`, `libmingwex`, `libmoldname`, import libraries) | 12.0.0 | ZPL-2.1, public domain and others (`licenses/mingw-w64-copyright`) | embedded in iscc, *in Windows programs* |
| GCC runtime (`libgcc`, `libgcc_eh`, `crtbegin.o`, `crtend.o`) | 14.2 | GPL-3.0-or-later WITH GCC-exception-3.1 | embedded in iscc, *in Windows programs* |
| glibc start files (`Scrt1.o`, `crti.o`, `crtn.o`, `libc_nonshared.a`) | 2.28 | LGPL-2.1-or-later, with the exception that permits linking them into any program (`licenses/glibc-COPYING.LIB`) | embedded in iscc, *in Linux programs* |
| wasi-libc (`crt1-command.o`, `libc.a`), with musl | of the Rust of the build | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | embedded in iscc, *in WebAssembly programs* |

The exceptions of LLVM and GCC permit distributing the programs that contain
these runtime parts under any terms.
