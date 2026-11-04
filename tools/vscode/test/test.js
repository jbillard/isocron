"use strict";
// The tests of the extension of VS Code (D267), run by run.sh:
//   node test.js REPOSITORY
// the functions of lib.js; then, with vscode-textmate and vscode-oniguruma
// (the engine of the grammars of VS Code), every Isocron file of the
// repository and every ```isc block of docs/language.md tokenized, nothing
// left open at its end, and the scopes of chosen tokens.

const assert = require("assert");
const fs = require("fs");
const path = require("path");

const root = process.argv[2] || path.join(__dirname, "..", "..", "..");
const ext = path.join(root, "tools", "vscode");
const lib = require(path.join(ext, "lib.js"));

let failed = 0;
let passed = 0;
async function test(name, f) {
    try {
        await f();
        passed++;
    } catch (e) {
        failed++;
        console.log("FAIL " + name + "\n  " + String(e && e.stack ? e.stack : e).split("\n").slice(0, 4).join("\n  "));
    }
}

async function libTests() {
    await test("a diagnostic of iscc check", () => {
        const d = lib.parseDiagnostics("/a/b.isc:2:16: error: unknown variable `y`\n");
        assert.deepStrictEqual(d, [{ file: "/a/b.isc", line: 2, column: 16, severity: "error", message: "unknown variable `y`" }]);
    });
    await test("a path of Windows, colors, a message on two lines", () => {
        const d = lib.parseDiagnostics("\x1b[1mC:\\x\\b.isc:3:5: error: expected `;`, found `return`\x1b[0m\r\n  the rest\r\nC:\\x\\m.isc:1:1: warning: w\r\n");
        assert.strictEqual(d.length, 2);
        assert.strictEqual(d[0].file, "C:\\x\\b.isc");
        assert.strictEqual(d[0].line, 3);
        assert.strictEqual(d[0].column, 5);
        assert.strictEqual(d[0].message, "expected `;`, found `return`\n  the rest");
        assert.strictEqual(d[1].severity, "warning");
    });
    await test("nothing for a file without error", () => {
        assert.deepStrictEqual(lib.parseDiagnostics(""), []);
    });
    await test("the declarations of a file", () => {
        const text = [
            "// function hidden(x: f64): f64 {",
            "/* type Hidden = A | B; */",
            "export type Shape = Circle(f64) | Square(f64);",
            "type Point = { x: f64, y: f64 };",
            "type Pair<T> = [T, T];",
            "const DT = 0.01;",
            "export const N: i64 = 3;",
            "declare function cos(x: f64): f64;",
            "export function area(s: Shape): f64 {",
            "    const local = 2.0;",
            '    const text = "function inside(";',
            "    return local;",
            "}",
            "action main(args: string[k]): i32 {",
            "    return 0;",
            "}",
        ].join("\n");
        const d = lib.declarations(text).map((x) => [x.name, x.kind, x.line, x.column]);
        assert.deepStrictEqual(d, [
            ["Shape", "type", 2, 12],
            ["Circle", "constructor", 2, 20],
            ["Square", "constructor", 2, 34],
            ["Point", "type", 3, 5],
            ["Pair", "type", 4, 5],
            ["DT", "const", 5, 6],
            ["N", "const", 6, 13],
            ["cos", "function", 7, 17],
            ["area", "function", 8, 16],
            ["main", "action", 13, 7],
        ]);
    });
    await test("the imports of a file", () => {
        const text = 'import * as geom from "./geom";\nimport { area, Shape as S } from "util";\n// import { x } from "./no";\n';
        assert.deepStrictEqual(lib.imports(text), ["./geom", "util"]);
    });
    await test("the program built of a file", () => {
        assert.strictEqual(lib.outputPath("/a/nbody.isc", false), "/a/nbody");
        assert.strictEqual(lib.outputPath("C:\\a\\nbody.isc", true), "C:\\a\\nbody.exe");
    });
    await test("the messages of the language server, in chunks of any size", () => {
        const msgs = [{ jsonrpc: "2.0", id: 1, result: { contents: { kind: "markdown", value: "`é` 😀" } } }, { jsonrpc: "2.0", method: "x", params: {} }];
        const bytes = Buffer.concat(msgs.map(lib.encodeMessage));
        assert.ok(lib.encodeMessage(msgs[0]).toString("utf8").startsWith("Content-Length: " + Buffer.byteLength(JSON.stringify(msgs[0])) + "\r\n\r\n"));
        for (const size of [1, 7, bytes.length]) {
            const got = [];
            const r = new lib.MessageReader((m) => got.push(m));
            for (let i = 0; i < bytes.length; i += size) r.push(bytes.slice(i, i + size));
            assert.deepStrictEqual(got, msgs, "chunks of " + size);
        }
    });
    await test("the path of the language server", () => {
        assert.strictEqual(lib.serverPath("", "iscc", "linux"), "isc-lsp");
        assert.strictEqual(lib.serverPath("", "/opt/isocron/iscc", "linux"), "/opt/isocron/isc-lsp");
        assert.strictEqual(lib.serverPath("", "C:\\isocron\\iscc.exe", "win32"), "C:\\isocron\\isc-lsp.exe");
        assert.strictEqual(lib.serverPath("", "iscc.exe", "win32"), "isc-lsp.exe");
        assert.strictEqual(lib.serverPath("/x/my-lsp", "/opt/isocron/iscc", "linux"), "/x/my-lsp");
    });
    await test("the folder of the workspace in the setting of the compiler", () => {
        assert.strictEqual(lib.expandFolder("${workspaceFolder}/target/release/iscc", "/w"), "/w/target/release/iscc");
        assert.strictEqual(lib.expandFolder("iscc", "/w"), "iscc");
        assert.strictEqual(lib.expandFolder("${workspaceFolder}/iscc", undefined), "${workspaceFolder}/iscc");
    });
    await test("the arguments of a run", () => {
        assert.deepStrictEqual(lib.splitArgs('  50000000 "a b"  c "d\\"e" '), ["50000000", "a b", "c", 'd"e']);
        assert.deepStrictEqual(lib.splitArgs(""), []);
    });
}

/** The Isocron files of the repository (not those of the builds). */
function isoFiles(dir, out) {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
        const p = path.join(dir, e.name);
        if (e.isDirectory()) {
            if (["target", ".git", "build", "bin", "upstream", "node_modules"].includes(e.name)) continue;
            isoFiles(p, out);
        } else if (e.name.endsWith(".isc")) {
            out.push(p);
        }
    }
    return out;
}

/** The ```isc blocks of a Markdown file. */
function blocks(md) {
    const out = [];
    const re = /^```(isc|isc-file[^\n]*)\n([\s\S]*?)^```/gm;
    let m;
    while ((m = re.exec(md))) out.push(m[2]);
    return out;
}

async function grammarTests() {
    let vsctm, onig;
    try {
        vsctm = require("vscode-textmate");
        onig = require("vscode-oniguruma");
    } catch (e) {
        console.log("vscode-textmate not found: the tests of the grammar are skipped (run.sh installs it)");
        return;
    }
    const wasm = fs.readFileSync(require.resolve("vscode-oniguruma/release/onig.wasm"));
    await onig.loadWASM(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength));
    const files = {
        "source.isocron": path.join(ext, "syntaxes", "isocron.tmLanguage.json"),
        "markdown.isocron.codeblock": path.join(ext, "syntaxes", "isocron.markdown.tmLanguage.json"),
    };
    const registry = new vsctm.Registry({
        onigLib: Promise.resolve({
            createOnigScanner: (s) => new onig.OnigScanner(s),
            createOnigString: (s) => new onig.OnigString(s),
        }),
        loadGrammar: async (scope) => (files[scope] ? vsctm.parseRawGrammar(fs.readFileSync(files[scope], "utf8"), files[scope]) : null),
    });
    const grammar = await registry.loadGrammar("source.isocron");
    const tokenize = (g, text) => {
        let stack = vsctm.INITIAL;
        const out = [];
        for (const line of text.split("\n")) {
            const r = g.tokenizeLine(line, stack);
            for (const t of r.tokens) out.push({ text: line.slice(t.startIndex, t.endIndex), scopes: t.scopes });
            stack = r.ruleStack;
        }
        return { out, stack };
    };
    // after the text, a line `x`: of the source alone if nothing is left open
    const closed = (g, text) => {
        const { stack } = tokenize(g, text);
        const r = g.tokenizeLine("x", stack);
        return r.tokens.length === 1 && r.tokens[0].scopes.length === 1;
    };
    const sources = isoFiles(root, []);
    await test("every Isocron file of the repository tokenized, nothing left open", () => {
        assert.ok(sources.length > 20, "only " + sources.length + " files");
        const open = sources.filter((f) => !closed(grammar, fs.readFileSync(f, "utf8")));
        assert.deepStrictEqual(open.map((f) => path.relative(root, f)), []);
    });
    const docs = blocks(fs.readFileSync(path.join(root, "docs", "language.md"), "utf8"));
    await test("every ```isc block of docs/language.md tokenized, nothing left open", () => {
        assert.ok(docs.length > 50, "only " + docs.length + " blocks");
        const open = docs.filter((b) => !closed(grammar, b));
        assert.deepStrictEqual(open.map((b) => b.split("\n")[0]), []);
    });
    const sample = [
        "export function area<T extends Num>(s: Shape, n: i64): f64 {",
        "    const a = 42u8 + 0xffn + 2.5e-3 + 1_000;",
        '    const t = "a\\nb" + `x ${n + 1} z`;',
        "    return Math.sqrt(map(iota(n), (i) => i * 2).length) as const; // a comment",
        "}",
        "type Shape = Circle(f64) | Square(f64);",
        "const DT = 0.01;",
        "/* a block",
        "   of comment */",
        "const b = xs.with(0, true) ?? false;",
    ].join("\n");
    const { out } = tokenize(grammar, sample);
    const scopeOf = (text, nth = 0) => {
        const all = out.filter((t) => t.text === text);
        assert.ok(all.length > nth, "no token " + JSON.stringify(text));
        return all[nth].scopes.join(" ");
    };
    const expect = [
        ["export", "storage.modifier.export"],
        ["function", "storage.type.function"],
        ["area", "entity.name.function"],
        ["extends", "keyword.other"],
        ["Shape", "entity.name.type"],
        ["i64", "support.type.primitive"],
        ["f64", "support.type.primitive"],
        ["const", "storage.type"],
        ["42u8", "constant.numeric.integer"],
        ["0xffn", "constant.numeric.hex"],
        ["2.5e-3", "constant.numeric.float"],
        ["1_000", "constant.numeric.integer"],
        ["\\n", "constant.character.escape"],
        ["${", "punctuation.definition.template-expression.begin"],
        ["return", "keyword.control"],
        ["Math", "support.class.math"],
        ["sqrt", "support.function.math"],
        ["map", "support.function.builtin"],
        ["iota", "support.function.builtin"],
        ["=>", "storage.type.function.arrow"],
        ["length", "support.variable.property"],
        ["as const", "keyword.other"],
        ["// a comment", "comment.line.double-slash"],
        ["Circle", "entity.name.type"],
        ["DT", "variable.other.constant"],
        ["   of comment ", "comment.block"],
        ["with", "entity.name.function.method"],
        ["true", "constant.language.boolean"],
        ["??", "keyword.operator"],
    ];
    for (const [text, scope] of expect) {
        await test("the scope of " + JSON.stringify(text), () => {
            const s = text === "// a comment" ? out.filter((t) => t.scopes.join(" ").includes("comment.line")).map((t) => t.scopes.join(" ")).join(" ") : scopeOf(text);
            assert.ok(s.includes(scope), JSON.stringify(text) + ": " + s);
        });
    }
    await test("a ```isc block of Markdown colored as Isocron", async () => {
        const md = await registry.loadGrammar("markdown.isocron.codeblock");
        const { out: m } = tokenize(md, "```isc\nconst a = 1;\n```\nafter");
        const c = m.find((t) => t.text === "const");
        assert.ok(c && c.scopes.join(" ").includes("storage.type.isocron"), JSON.stringify(c));
        // after the block: no scope of the grammar of Isocron (only the
        // root of this one, loaded alone here)
        const after = m.find((t) => t.text === "after");
        assert.ok(after && after.scopes.every((s) => s === "markdown.isocron.codeblock"), JSON.stringify(after));
    });
}

(async () => {
    await libTests();
    await grammarTests();
    console.log(`${passed} passed, ${failed} failed`);
    process.exit(failed ? 1 : 0);
})();
