"use strict";
// The extension of VS Code for Isocron (D267): the errors of `iscc check` in
// the editor when a file is opened or saved, the commands that check, run,
// build and show the IR of a file, the outline of a file and the definition
// of a name (in the file and in the modules it imports); the hover from the
// language server `isc-lsp` (D269: the type of a name, the comments of a
// declaration, the documentation of the standard library). The compiler is
// `iscc` from the PATH, or the setting `isocron.iscc`; the server, the
// `isc-lsp` beside it, or the setting `isocron.languageServer`.

const vscode = require("vscode");
const cp = require("child_process");
const fs = require("fs");
const path = require("path");
const lib = require("./lib");

let diagnostics;
let channel;
let warnedMissing = false;
// the files whose diagnostics each checked file set, to clear them again
const reported = new Map();

function config() {
    return vscode.workspace.getConfiguration("isocron");
}

function compiler() {
    const folders = (vscode.workspace.workspaceFolders || []).map((f) => f.uri.fsPath);
    return lib.expandFolder(config().get("iscc") || "iscc", folders[0]);
}

/** `iscc ARGS` in `cwd`: { code, stdout, stderr, missing }. */
function runIscc(args, cwd) {
    return new Promise((resolve) => {
        cp.execFile(compiler(), args, { cwd, timeout: 60000, maxBuffer: 64 << 20 }, (err, stdout, stderr) => {
            if (err && err.code === "ENOENT") {
                resolve({ code: -1, stdout: "", stderr: "", missing: true });
                return;
            }
            const code = err ? (typeof err.code === "number" ? err.code : 1) : 0;
            resolve({ code, stdout: String(stdout), stderr: String(stderr), missing: false });
        });
    });
}

function warnMissing() {
    if (warnedMissing) return;
    warnedMissing = true;
    vscode.window
        .showWarningMessage(`Isocron: the compiler \`${compiler()}\` was not found. Set "isocron.iscc" to its path.`, "Open settings")
        .then((choice) => {
            if (choice) vscode.commands.executeCommand("workbench.action.openSettings", "isocron.iscc");
        });
}

function isIsocron(document) {
    return document && document.languageId === "isocron" && document.uri.scheme === "file";
}

/** The range of the word at a position of a diagnostic (one character when none). */
function rangeAt(document, line, column) {
    const pos = new vscode.Position(Math.max(0, line - 1), Math.max(0, column - 1));
    if (document) {
        const word = document.getWordRangeAtPosition(pos);
        if (word) return word;
    }
    return new vscode.Range(pos, pos.translate(0, 1));
}

/** Checks a saved file with `iscc check` and shows its errors. */
async function check(document) {
    if (!isIsocron(document)) return;
    const file = document.uri.fsPath;
    const dir = path.dirname(file);
    const r = await runIscc(["check", file, "--color", "never"], dir);
    if (r.missing) {
        warnMissing();
        return;
    }
    const byFile = new Map([[file, []]]);
    const found = lib.parseDiagnostics(r.stdout + "\n" + r.stderr);
    for (const d of found) {
        const f = path.resolve(dir, d.file);
        const doc = vscode.workspace.textDocuments.find((t) => t.uri.fsPath === f);
        const severity =
            d.severity === "error"
                ? vscode.DiagnosticSeverity.Error
                : d.severity === "warning"
                ? vscode.DiagnosticSeverity.Warning
                : vscode.DiagnosticSeverity.Information;
        const diag = new vscode.Diagnostic(rangeAt(doc, d.line, d.column), d.message, severity);
        diag.source = "iscc";
        if (!byFile.has(f)) byFile.set(f, []);
        byFile.get(f).push(diag);
    }
    if (r.code !== 0 && found.length === 0) {
        const text = (r.stderr || r.stdout).trim() || `iscc check exited with code ${r.code}`;
        byFile.get(file).push(new vscode.Diagnostic(new vscode.Range(0, 0, 0, 1), text, vscode.DiagnosticSeverity.Error));
    }
    // the files of the previous check of this one, now without errors
    for (const f of reported.get(file) || []) {
        if (!byFile.has(f)) diagnostics.delete(vscode.Uri.file(f));
    }
    for (const [f, list] of byFile) diagnostics.set(vscode.Uri.file(f), list);
    reported.set(file, new Set(byFile.keys()));
}

/** The active Isocron file, saved. */
async function activeFile() {
    const editor = vscode.window.activeTextEditor;
    if (!editor || !isIsocron(editor.document)) {
        vscode.window.showInformationMessage("Isocron: open a .isc file first.");
        return null;
    }
    if (editor.document.isDirty) await editor.document.save();
    return editor.document;
}

/** The arguments of a run of `file`, asked, the last ones proposed. */
async function askArgs(context, file) {
    const key = "isocron.args:" + file;
    const value = await vscode.window.showInputBox({
        prompt: "Arguments of the program (the args of main)",
        value: context.workspaceState.get(key, ""),
    });
    if (value === undefined) return undefined;
    await context.workspaceState.update(key, value);
    return lib.splitArgs(value);
}

/** Runs `command args` as a task in the terminal, the errors of iscc matched. */
function runTask(name, command, args, cwd) {
    const task = new vscode.Task(
        { type: "isocron", task: name },
        vscode.TaskScope.Workspace,
        name,
        "isocron",
        new vscode.ProcessExecution(command, args, { cwd }),
        ["$iscc"]
    );
    task.presentationOptions = { reveal: vscode.TaskRevealKind.Always, clear: true, focus: false };
    return vscode.tasks.executeTask(task);
}

/** The exit code of a task started by `runTask`. */
function exitOf(execution) {
    return new Promise((resolve) => {
        const sub = vscode.tasks.onDidEndTaskProcess((e) => {
            if (e.execution === execution) {
                sub.dispose();
                resolve(e.exitCode);
            }
        });
    });
}

function buildArgs(file) {
    const out = lib.outputPath(file, process.platform === "win32");
    return { out, args: ["build", file, "-o", out, ...(config().get("buildArgs") || [])] };
}

async function commandCheck() {
    const doc = await activeFile();
    if (doc) await check(doc);
}

async function commandRun(context) {
    const doc = await activeFile();
    if (!doc) return;
    const file = doc.uri.fsPath;
    const args = await askArgs(context, file);
    if (args === undefined) return;
    await runTask("run " + path.basename(file), compiler(), ["run", file, ...args], path.dirname(file));
}

async function commandBuild() {
    const doc = await activeFile();
    if (!doc) return;
    const file = doc.uri.fsPath;
    const b = buildArgs(file);
    await runTask("build " + path.basename(file), compiler(), b.args, path.dirname(file));
}

async function commandBuildAndRun(context) {
    const doc = await activeFile();
    if (!doc) return;
    const file = doc.uri.fsPath;
    const args = await askArgs(context, file);
    if (args === undefined) return;
    const b = buildArgs(file);
    const exec = await runTask("build " + path.basename(file), compiler(), b.args, path.dirname(file));
    const code = await exitOf(exec);
    if (code !== 0) return;
    await runTask(path.basename(b.out), b.out, args, path.dirname(file));
}

async function commandShowIr() {
    const doc = await activeFile();
    if (!doc) return;
    const file = doc.uri.fsPath;
    const r = await runIscc(["ir", file, "--color", "never"], path.dirname(file));
    if (r.missing) {
        warnMissing();
        return;
    }
    if (r.code !== 0) {
        channel.clear();
        channel.append(r.stderr || r.stdout);
        channel.show(true);
        return;
    }
    const ir = await vscode.workspace.openTextDocument({ content: r.stdout, language: "plaintext" });
    await vscode.window.showTextDocument(ir, { viewColumn: vscode.ViewColumn.Beside, preview: true });
}

const KINDS = {
    function: vscode.SymbolKind.Function,
    action: vscode.SymbolKind.Event,
    type: vscode.SymbolKind.Struct,
    constructor: vscode.SymbolKind.EnumMember,
    const: vscode.SymbolKind.Constant,
};

const symbols = {
    provideDocumentSymbols(document) {
        return lib.declarations(document.getText()).map((d) => {
            const range = new vscode.Range(d.line, d.column, d.line, d.column + d.name.length);
            return new vscode.DocumentSymbol(d.name, d.kind, KINDS[d.kind], document.lineAt(d.line).range, range);
        });
    },
};

/** The files a file imports, resolved (`./x` next to it, others in $ISC_PATH). */
function importedFiles(file, text) {
    const out = [];
    const roots = (process.env.ISC_PATH || "").split(path.delimiter).filter(Boolean);
    for (const m of lib.imports(text)) {
        const name = m.endsWith(".isc") ? m : m + ".isc";
        const relative = m.startsWith("./") || m.startsWith("../");
        const candidates = relative ? [path.resolve(path.dirname(file), name)] : roots.map((r) => path.resolve(r, name));
        const found = candidates.find((c) => fs.existsSync(c));
        if (found) out.push(found);
    }
    return out;
}

const definitions = {
    provideDefinition(document, position) {
        const range = document.getWordRangeAtPosition(position, /[A-Za-z_][A-Za-z0-9_]*/);
        if (!range) return undefined;
        const word = document.getText(range);
        const at = (uri, d) => new vscode.Location(uri, new vscode.Position(d.line, d.column));
        const here = lib.declarations(document.getText()).filter((d) => d.name === word);
        if (here.length > 0) return here.map((d) => at(document.uri, d));
        const out = [];
        for (const f of importedFiles(document.uri.fsPath, document.getText())) {
            let text;
            try {
                text = fs.readFileSync(f, "utf8");
            } catch (e) {
                continue;
            }
            for (const d of lib.declarations(text)) if (d.name === word) out.push(at(vscode.Uri.file(f), d));
        }
        return out;
    },
};

// ---- the language server (D269): the hover, from `isc-lsp`, the server of
// every editor; the other features of this extension work without it

let server = null;

function folder() {
    return ((vscode.workspace.workspaceFolders || [])[0] || {}).uri;
}

/** `isc-lsp` started once (again after a crash or a change of the settings). */
function startServer() {
    if (server) return server;
    const ws = folder();
    const cmd = lib.serverPath(lib.expandFolder(config().get("languageServer") || "", ws && ws.fsPath), compiler(), process.platform);
    const s = { cmd, pending: new Map(), next: 1, opened: new Map(), dead: false, initialized: false };
    server = s;
    const gone = (why) => {
        if (s.dead) return;
        s.dead = true;
        channel.appendLine(`isc-lsp (${cmd}): ${why}`);
        for (const done of s.pending.values()) done(null);
        s.pending.clear();
        // a server that worked starts again at the next hover; one that
        // could not start, at a change of the settings
        if (s.initialized && server === s) server = null;
    };
    try {
        s.proc = cp.spawn(cmd, [], { cwd: ws && ws.fsPath, stdio: ["pipe", "pipe", "pipe"] });
    } catch (e) {
        gone(e.message);
        s.ready = Promise.resolve(false);
        return s;
    }
    const reader = new lib.MessageReader((m) => {
        // the answers; the notifications (its diagnostics) are those of iscc check here
        if (m.method === undefined && s.pending.has(m.id)) {
            const done = s.pending.get(m.id);
            s.pending.delete(m.id);
            done(m.error ? null : m.result);
        }
    });
    s.proc.stdout.on("data", (c) => reader.push(c));
    s.proc.stderr.on("data", (c) => channel.append(String(c)));
    s.proc.stdin.on("error", () => {});
    s.proc.on("error", (e) => gone(e.code === "ENOENT" ? "not found: the hover needs isc-lsp, beside iscc or on the PATH (setting isocron.languageServer)" : e.message));
    s.proc.on("exit", (code) => gone("exited with the code " + code));
    s.send = (msg) => {
        if (!s.dead) s.proc.stdin.write(lib.encodeMessage(msg));
    };
    s.request = (method, params) =>
        new Promise((resolve) => {
            if (s.dead) return resolve(null);
            const id = s.next++;
            const timer = setTimeout(() => {
                s.pending.delete(id);
                resolve(null);
            }, 10000);
            s.pending.set(id, (r) => {
                clearTimeout(timer);
                resolve(r);
            });
            s.send({ jsonrpc: "2.0", id, method, params });
        });
    s.notify = (method, params) => s.send({ jsonrpc: "2.0", method, params });
    s.ready = s.request("initialize", { processId: process.pid, rootUri: ws ? ws.toString() : null, capabilities: {} }).then((r) => {
        if (!r) return false;
        s.initialized = true;
        s.notify("initialized", {});
        return true;
    });
    return s;
}

function stopServer() {
    const s = server;
    server = null;
    if (!s || s.dead) return Promise.resolve();
    const kill = () => {
        if (!s.dead) s.proc.kill();
    };
    return Promise.race([s.request("shutdown", null), new Promise((r) => setTimeout(r, 2000))]).then(() => {
        s.notify("exit", null);
        setTimeout(kill, 1000);
    });
}

/** The text of `document` given to the server, as the editor has it (saved or not). */
function sync(s, document) {
    const uri = document.uri.toString();
    const v = s.opened.get(uri);
    if (v === undefined) {
        s.notify("textDocument/didOpen", { textDocument: { uri, languageId: "isocron", version: document.version, text: document.getText() } });
    } else if (v !== document.version) {
        s.notify("textDocument/didChange", { textDocument: { uri, version: document.version }, contentChanges: [{ text: document.getText() }] });
    }
    s.opened.set(uri, document.version);
    return uri;
}

const hovers = {
    async provideHover(document, position) {
        const s = startServer();
        if (!(await s.ready)) return undefined;
        const uri = sync(s, document);
        const r = await s.request("textDocument/hover", { textDocument: { uri }, position: { line: position.line, character: position.character } });
        if (!r || !r.contents) return undefined;
        const md = new vscode.MarkdownString(typeof r.contents === "string" ? r.contents : r.contents.value);
        const range = r.range && new vscode.Range(r.range.start.line, r.range.start.character, r.range.end.line, r.range.end.character);
        return new vscode.Hover(md, range);
    },
};

function activate(context) {
    diagnostics = vscode.languages.createDiagnosticCollection("isocron");
    channel = vscode.window.createOutputChannel("Isocron");
    const selector = { language: "isocron", scheme: "file" };
    context.subscriptions.push(
        diagnostics,
        channel,
        vscode.commands.registerCommand("isocron.check", commandCheck),
        vscode.commands.registerCommand("isocron.run", () => commandRun(context)),
        vscode.commands.registerCommand("isocron.build", commandBuild),
        vscode.commands.registerCommand("isocron.buildAndRun", () => commandBuildAndRun(context)),
        vscode.commands.registerCommand("isocron.showIr", commandShowIr),
        vscode.languages.registerDocumentSymbolProvider(selector, symbols),
        vscode.languages.registerDefinitionProvider(selector, definitions),
        vscode.languages.registerHoverProvider(selector, hovers),
        vscode.workspace.onDidSaveTextDocument((d) => {
            if (config().get("checkOnSave")) check(d);
        }),
        vscode.workspace.onDidOpenTextDocument((d) => {
            if (config().get("checkOnOpen")) check(d);
        }),
        vscode.workspace.onDidChangeConfiguration((e) => {
            if (e.affectsConfiguration("isocron.iscc") || e.affectsConfiguration("isocron.languageServer")) stopServer();
        }),
        vscode.workspace.onDidCloseTextDocument((d) => {
            if (!isIsocron(d)) return;
            if (server && server.opened.delete(d.uri.toString())) server.notify("textDocument/didClose", { textDocument: { uri: d.uri.toString() } });
            for (const f of reported.get(d.uri.fsPath) || []) diagnostics.delete(vscode.Uri.file(f));
            reported.delete(d.uri.fsPath);
        })
    );
    if (config().get("checkOnOpen")) vscode.workspace.textDocuments.forEach((d) => check(d));
}

function deactivate() {
    return stopServer();
}

module.exports = { activate, deactivate };
