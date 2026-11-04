"use strict";
// The test of the extension inside VS Code (D267), run by windows.py:
//   Code.exe --extensionDevelopmentPath=EXT --extensionTestsPath=THIS WORKSPACE
// in the folder workspace/ (a correct file, a wrong one, one fixed during the
// test, a module and the file that imports it; `isocron.iscc` names the
// iscc.exe next to it through `${workspaceFolder}`). Each step is written to
// results.json beside this file; a step that fails makes the run fail.

const vscode = require("vscode");
const fs = require("fs");
const path = require("path");

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** The value of `f` once it is truthy, tried for `ms` milliseconds. */
async function waitFor(what, f, ms = 60000) {
    const t0 = Date.now();
    while (Date.now() - t0 < ms) {
        const v = await f();
        if (v) return v;
        await sleep(200);
    }
    throw new Error("timed out waiting for " + what);
}

exports.run = async function () {
    const results = [];
    const ws = vscode.workspace.workspaceFolders[0].uri.fsPath;
    const step = async (name, f) => {
        try {
            results.push({ name, ok: true, detail: String(await f()) });
        } catch (e) {
            results.push({ name, ok: false, detail: String((e && e.stack) || e) });
        }
    };
    const open = async (name) => {
        const doc = await vscode.workspace.openTextDocument(path.join(ws, name));
        await vscode.window.showTextDocument(doc);
        return doc;
    };
    const errors = (doc) => vscode.languages.getDiagnostics(doc.uri);

    await step("the extension is there and active", async () => {
        const e = vscode.extensions.getExtension("isocron.isocron");
        if (!e) throw new Error("no extension isocron.isocron");
        await e.activate();
        return "isocron.isocron " + e.packageJSON.version;
    });
    await step("a .isc file is in the language isocron", async () => {
        const d = await open("ok.isc");
        if (d.languageId !== "isocron") throw new Error(d.languageId);
        return d.languageId;
    });
    await step("the error of iscc check at its place", async () => {
        const d = await open("err.isc");
        const x = (await waitFor("the error", () => errors(d).length > 0 && errors(d)))[0];
        const at = `${x.range.start.line + 1}:${x.range.start.character + 1}`;
        if (at !== "2:16" || !x.message.includes("unknown variable") || x.severity !== vscode.DiagnosticSeverity.Error) {
            throw new Error(at + " " + x.message);
        }
        return at + " " + x.message + " (" + d.getText(x.range) + ")";
    });
    await step("no error in a correct file", async () => {
        const d = await open("main.isc");
        await sleep(5000);
        if (errors(d).length) throw new Error(errors(d).map((x) => x.message).join("; "));
        return "none";
    });
    await step("an error fixed and saved is gone", async () => {
        const d = await open("fix.isc");
        await waitFor("the error", () => errors(d).length > 0);
        const line = d.lineAt(1);
        const edit = new vscode.WorkspaceEdit();
        edit.replace(d.uri, line.range, "    return x + x;");
        await vscode.workspace.applyEdit(edit);
        await d.save();
        await waitFor("no error", () => errors(d).length === 0);
        return "cleared";
    });
    await step("the outline of a file", async () => {
        const d = await open("main.isc");
        const syms = await waitFor("the symbols", () => vscode.commands.executeCommand("vscode.executeDocumentSymbolProvider", d.uri));
        const names = syms.map((s) => s.name).join(",");
        if (names !== "twice,main") throw new Error(names);
        return names;
    });
    await step("the definition of a name of an imported module", async () => {
        const d = await open("main.isc");
        const line = d.getText().split("\n").findIndex((l) => l.includes("print(area("));
        const col = d.lineAt(line).text.indexOf("area(") + 1;
        const locs = await vscode.commands.executeCommand("vscode.executeDefinitionProvider", d.uri, new vscode.Position(line, col));
        const l = (locs || [])[0];
        const where = l ? path.basename((l.uri || l.targetUri).fsPath) + ":" + ((l.range || l.targetRange).start.line + 1) : "none";
        if (where !== "geom.isc:2") throw new Error(where);
        return where;
    });
    await step("the definition of a name of the file", async () => {
        const d = await open("main.isc");
        const line = d.getText().split("\n").findIndex((l) => l.includes("twice(1.0)"));
        const col = d.lineAt(line).text.indexOf("twice(") + 1;
        const locs = await vscode.commands.executeCommand("vscode.executeDefinitionProvider", d.uri, new vscode.Position(line, col));
        const l = (locs || [])[0];
        const where = l ? path.basename((l.uri || l.targetUri).fsPath) + ":" + ((l.range || l.targetRange).start.line + 1) : "none";
        if (where !== "main.isc:3") throw new Error(where);
        return where;
    });
    // the text of the hovers at the first `word` of the line that holds `text`
    const hoverAt = async (name, text, word) => {
        const d = await open(name);
        const line = d.getText().split("\n").findIndex((l) => l.includes(text));
        const col = d.lineAt(line).text.indexOf(word, d.lineAt(line).text.indexOf(text)) + 1;
        const hs = (await vscode.commands.executeCommand("vscode.executeHoverProvider", d.uri, new vscode.Position(line, col))) || [];
        return hs.map((h) => h.contents.map((c) => (typeof c === "string" ? c : c.value)).join("\n")).join("\n");
    };
    await step("the hover of a function of an imported module: its signature, its comment", async () => {
        const t = await hoverAt("main.isc", "print(area(", "area");
        if (!t.includes("function area(r: f64): f64") || !t.includes("what it exports is imported")) throw new Error(t);
        return "function area(r: f64): f64 + its comment";
    });
    await step("the hover of a parameter: its type", async () => {
        const t = await hoverAt("main.isc", "return 2.0 * x", "x");
        if (!t.includes("x: f64") || !t.includes("parameter")) throw new Error(t);
        return "x: f64";
    });
    await step("the hover of a function of the standard library: its documentation", async () => {
        const t = await hoverAt("main.isc", "print(area(", "print");
        if (!t.includes("`print(") || !t.includes("docs/language.md#")) throw new Error(t);
        return t.split("\n")[0];
    });
    await step("the hover of a text typed, not saved: the type of a new constant", async () => {
        const d = await open("main.isc");
        const line = d.getText().split("\n").findIndex((l) => l.includes("return 2.0 * x"));
        const edit = new vscode.WorkspaceEdit();
        edit.insert(d.uri, new vscode.Position(line, 0), "    const q = x * 3.0;\n");
        await vscode.workspace.applyEdit(edit);
        try {
            const t = await hoverAt("main.isc", "const q = x", "q");
            if (!d.isDirty || !t.includes("q: f64")) throw new Error(t);
            return "q: f64 (unsaved)";
        } finally {
            await vscode.commands.executeCommand("workbench.action.files.revert");
        }
    });
    await step("the IR of a file", async () => {
        await open("ok.isc");
        await vscode.commands.executeCommand("isocron.showIr");
        const ed = await waitFor("the IR", () => vscode.window.visibleTextEditors.find((e) => e.document.isUntitled && e.document.getText().includes("fold")));
        return ed.document.getText().split("\n").length + " lines";
    });
    await step("a file built by the command, its program run", async () => {
        await open("ok.isc");
        const exe = path.join(ws, "ok.exe");
        if (fs.existsSync(exe)) fs.unlinkSync(exe);
        const ended = new Promise((resolve) => {
            const sub = vscode.tasks.onDidEndTaskProcess((e) => {
                sub.dispose();
                resolve(e.exitCode);
            });
        });
        await vscode.commands.executeCommand("isocron.build");
        const code = await ended;
        if (code !== 0 || !fs.existsSync(exe)) throw new Error("exit code " + code);
        const out = require("child_process").execFileSync(exe, [], { encoding: "utf8" }).trim();
        if (out !== "45") throw new Error("output " + out);
        return "ok.exe: " + out;
    });

    fs.writeFileSync(path.join(__dirname, "results.json"), JSON.stringify(results, null, 2));
    const failed = results.filter((r) => !r.ok);
    if (failed.length) throw new Error(failed.length + " step(s) failed");
};
