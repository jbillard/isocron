#!/usr/bin/env bash
# The archives of a release (D249):
#   package.sh VERSION ISCC_LINUX ISCC_WINDOWS OUT
# OUT/isocron-VERSION-x86_64-linux.tar.gz and OUT/isocron-VERSION-x86_64-windows.zip,
# each with the compiler, the language server (isc-lsp, beside the compiler
# built by build_iscc.sh, D269), LICENSE, THIRD_PARTY_LICENSES.md and the texts of
# the licenses, README.md, CHANGELOG.md, docs/language.md and the examples;
# OUT/SHA256SUMS. The texts: $ISC_LLVM_SRC (llvm-project sources),
# $ISC_MINGW_DIR (MinGW-w64, with its share/doc), $ISC_GLIBC_LICENSE (the
# COPYING.LIB of glibc), and the licenses of the Rust toolchain.
set -euo pipefail
v=$1
linux=$(realpath "$2")
windows=$(realpath "$3")
out=$(realpath -m "$4")
root="$(cd "$(dirname "$0")/../.." && pwd)"
: "${ISC_LLVM_SRC:?}" "${ISC_MINGW_DIR:?}" "${ISC_GLIBC_LICENSE:?}"
rustlic="$(rustc --print sysroot)/share/doc/rust/licenses"
mkdir -p "$out"
stage() {
    local d="$out/isocron-$v-x86_64-$1"
    mkdir -p "$d/docs" "$d/licenses"
    cp "$root/LICENSE" "$root/README.md" "$root/CHANGELOG.md" "$d/"
    cp "$root/tools/release/THIRD_PARTY_LICENSES.md" "$d/"
    cp "$root/docs/language.md" "$d/docs/"
    cp -r "$root/examples" "$d/"
    cp "$ISC_LLVM_SRC/llvm/LICENSE.TXT" "$d/licenses/LLVM-LICENSE.TXT"
    cp "$ISC_MINGW_DIR/share/doc/mingw-w64-x86-64-dev/copyright" "$d/licenses/mingw-w64-copyright"
    cp "$ISC_GLIBC_LICENSE" "$d/licenses/glibc-COPYING.LIB"
    for f in Apache-2.0.txt MIT.txt LLVM-exception.txt GCC-exception-3.1.txt GPL-3.0-or-later.txt; do
        cp "$rustlic/$f" "$d/licenses/"
    done
    echo "$d"
}
d=$(stage linux)
cp "$linux" "$d/iscc"
cp "$(dirname "$linux")/isc-lsp" "$d/isc-lsp"
tar -C "$out" -czf "$out/isocron-$v-x86_64-linux.tar.gz" "isocron-$v-x86_64-linux"
d=$(stage windows)
cp "$windows" "$d/iscc.exe"
cp "$(dirname "$windows")/isc-lsp.exe" "$d/isc-lsp.exe"
(cd "$out" && python3 -c "
import os, sys, zipfile
name = sys.argv[1]
with zipfile.ZipFile(name + '.zip', 'w', zipfile.ZIP_DEFLATED) as z:
    for base, dirs, files in os.walk(name):
        for f in sorted(files):
            z.write(os.path.join(base, f))
" "isocron-$v-x86_64-windows")
(cd "$out" && sha256sum "isocron-$v-x86_64-linux.tar.gz" "isocron-$v-x86_64-windows.zip" > SHA256SUMS)
cat "$out/SHA256SUMS"
