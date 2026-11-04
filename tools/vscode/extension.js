"use strict";
// The extension of VS Code for Isocron (D267): the errors of `iscc check` in
// the editor when a file is opened or saved, the commands that check, run,
// build and show the IR of a file, the outline of a file and the definition
// of a name (in the file and in the modules it imports). The compiler is
// `iscc` from the PATH, or the setting `isocron.iscc`.

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
        vscode.workspace.onDidSaveTextDocument((d) => {
            if (config().get("checkOnSave")) check(d);
        }),
        vscode.workspace.onDidOpenTextDocument((d) => {
            if (config().get("checkOnOpen")) check(d);
        }),
        vscode.workspace.onDidCloseTextDocument((d) => {
            if (!isIsocron(d)) return;
            for (const f of reported.get(d.uri.fsPath) || []) diagnostics.delete(vscode.Uri.file(f));
            reported.delete(d.uri.fsPath);
        })
    );
    if (config().get("checkOnOpen")) vscode.workspace.textDocuments.forEach((d) => check(d));
}

function deactivate() {}

module.exports = { activate, deactivate };
