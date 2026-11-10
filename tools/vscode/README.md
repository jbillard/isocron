# Isocron for VS Code

The extension of VS Code for the Isocron language (`.isc` files, DECISIONS
D267). It uses the compiler `iscc` of the machine where the files are, and
for the hover the language server `isc-lsp` beside it (D269): those of a
release (`isocron-VERSION-x86_64-linux.tar.gz`,
`isocron-VERSION-x86_64-windows.zip`; `isc-lsp` in those from 0.2.1) or
built from this repository (`cargo build --release` makes both).

| Feature | How |
| --- | --- |
| colors | the keywords, types, literals (`42u8`, `0xffn`, `2.5e-3`), strings and templates, comments; also the ` ```isc ` blocks of Markdown files (`docs/language.md`) |
| errors in the editor | `iscc check` when a file is opened and saved: the error at its line and column, in the file it is in (an imported module too) |
| run | **Isocron: Run this file (interpreter)**: `iscc run FILE ARGS`, the arguments asked (the last ones proposed) |
| build | **Isocron: Build this file**: `iscc build FILE -o FILE-without-.isc` and the options of `isocron.buildArgs`; **Build and run this file** (also the ▷ button of the editor) runs the program after |
| the IR | **Isocron: Show the IR of this file**: `iscc ir FILE` beside the source |
| hover | from `isc-lsp`, on the text as typed: the type of the name under the pointer (a parameter, a constant, a size, a field), the signature of a function where it is declared or called and the comments above its declaration (also in an imported module), the documentation of a function of the standard library with a link to its section ([the language server](../../docs/language.md#editors-the-language-server-isc-lsp)) |
| outline, definitions | the functions, actions, types, constructors and constants of a file (outline, breadcrumbs, `Ctrl+Shift+O`); **Go to Definition** in the file and in the modules it imports (`./geom`, or `$ISC_PATH`) |
| snippets | `main`, `function`, `functiont` (generic), `action`, `typer` (record), `types` (sum), `match`, `map`, `mapi`, `reduce`, `fold`, `filter`, `scan`, `import`, `importall` |
| tasks | the problem matcher `$iscc`, for the `iscc` commands of a `tasks.json` |

The errors come from the file on disk: a file is checked when it is saved,
not while it is typed.

## Install

Download `isocron-VERSION.vsix` from the
[releases](https://github.com/jbillard/isocron/releases) (from 0.2.1, beside
the archives), or make it (Python 3 only, no npm):

```sh
python3 tools/vscode/package.py          # tools/vscode/isocron-0.6.0.vsix
```

then in VS Code: Extensions, `...`, **Install from VSIX...**, or
`code --install-extension isocron-0.6.0.vsix`.

* **On the machine of the files**: put `iscc` (or `iscc.exe`) in the PATH,
  or set `isocron.iscc` to its path; `isc-lsp` beside it (or in the PATH)
  for the hover. Without it, the rest works.
* **With Remote - SSH** (the files on a server): install the package in the
  window connected to the server (the extension runs there, by
  `extensionKind`); `iscc` is the one of the server.

## Settings

| Setting | Default | |
| --- | --- | --- |
| `isocron.iscc` | `iscc` | the compiler: a name in the PATH or a path; `${workspaceFolder}` is the folder opened (this repository sets `${workspaceFolder}/target/release/iscc` in `.vscode/settings.json`) |
| `isocron.checkOnSave` | `true` | check a file when it is saved |
| `isocron.checkOnOpen` | `true` | check a file when it is opened |
| `isocron.languageServer` | (empty) | the language server of the hover: empty for the `isc-lsp` beside `isocron.iscc` when it is a path, else from the PATH; or its path (`${workspaceFolder}` too) |
| `isocron.buildArgs` | `[]` | more options of `iscc build`: `["--isa", "avx2"]`, `["--target", "x86_64-windows"]` |

## Tests

`tools/vscode/test/run.sh` runs `test/test.js` in a Docker image of Node.js:
the functions of `lib.js` (the messages of `iscc`, the declarations, the
imports, the arguments, the messages of the language server and its path), and every Isocron file of the repository and every
` ```isc ` block of `docs/language.md` tokenized by the grammar with
`vscode-textmate` (the engine of VS Code), with the scopes of chosen
tokens checked.

`tools/vscode/test/windows.py VSCODE_ZIP ISCC_EXE` tries it in a real VS
Code on the Windows machine of `tools/windows/run.py` (a portable VS Code
from its `win32-x64-archive`, kept there): the `.vsix` installed by the
command line of VS Code, then `test/integration/index.js` run inside it on
the files of the package (`--extensionTestsPath`): the language of a
`.isc` file, an error at its place and gone once fixed and saved, none in a
correct file, the outline, the definitions in the file and in a module it
imports, the hovers (a function of a module with its comment, a parameter,
a function of the standard library, a constant typed and not saved), the
IR, a file built and its program run (`ISCC_EXE` with the `isc-lsp.exe`
beside it).
