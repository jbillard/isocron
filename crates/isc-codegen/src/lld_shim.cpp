// LLD in the process (D248): the linkers of ELF, COFF, MinGW and
// WebAssembly that iscc uses, called by `isc_codegen::lld::link`. LLD keeps
// state between links: the caller runs one link at a time.
#include "lld/Common/Driver.h"
#include "llvm/Support/raw_ostream.h"
#include <vector>

LLD_HAS_DRIVER(elf)
LLD_HAS_DRIVER(coff)
LLD_HAS_DRIVER(mingw)
LLD_HAS_DRIVER(wasm)

extern "C" int isc_lld_link(int argc, const char **argv) {
    std::vector<const char *> args(argv, argv + argc);
    lld::Result r = lld::lldMain(args, llvm::outs(), llvm::errs(),
                                 {{lld::WinLink, &lld::coff::link},
                                  {lld::Gnu, &lld::elf::link},
                                  {lld::MinGW, &lld::mingw::link},
                                  {lld::Wasm, &lld::wasm::link}});
    llvm::outs().flush();
    llvm::errs().flush();
    return r.retCode;
}
