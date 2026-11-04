"use strict";
// The parts of the extension that do not need VS Code (tested by
// test/test.js): the messages of `iscc check`, the declarations and the
// imports of a file, the path of a program built, the arguments of a run,
// the messages of the language server and its path.

const path = require("path");

/**
 * The diagnostics of the output of `iscc check FILE --color never`: lines
 * `PATH:LINE:COLUMN: error: MESSAGE` (also `warning`, `note`); the lines
 * after one that are not themselves a diagnostic belong to its message. A
 * path may hold colons (`C:\dir\a.isc` on Windows): the line and the column
 * are the last two numbers before the severity.
 */
function parseDiagnostics(output) {
    const re = /^(.*?):(\d+):(\d+): (error|warning|note): (.*)$/;
    const out = [];
    for (const raw of output.split(/\r?\n/)) {
        const line = raw.replace(/\x1b\[[0-9;]*m/g, "");
        const m = re.exec(line);
        if (m) {
            out.push({ file: m[1], line: Number(m[2]), column: Number(m[3]), severity: m[4], message: m[5] });
        } else if (out.length > 0 && line.trim() !== "") {
            out[out.length - 1].message += "\n" + line.trimEnd();
        }
    }
    return out;
}

/** `text` with its comments replaced by spaces (the positions kept). */
function blankComments(text) {
    let out = "";
    let i = 0;
    let quote = null;
    while (i < text.length) {
        const c = text[i];
        if (quote) {
            out += c;
            if (c === "\\" && i + 1 < text.length) {
                out += text[i + 1];
                i += 2;
                continue;
            }
            if (c === quote || (c === "\n" && quote !== "`")) quote = null;
            i++;
        } else if (c === '"' || c === "'" || c === "`") {
            quote = c;
            out += c;
            i++;
        } else if (text.startsWith("//", i)) {
            while (i < text.length && text[i] !== "\n") {
                out += " ";
                i++;
            }
        } else if (text.startsWith("/*", i)) {
            const end = text.indexOf("*/", i + 2);
            const stop = end < 0 ? text.length : end + 2;
            for (; i < stop; i++) out += text[i] === "\n" ? "\n" : " ";
        } else {
            out += c;
            i++;
        }
    }
    return out;
}

/**
 * The declarations of the top level of a file: functions, actions, types and
 * the constructors of the sum types written on one line, constants (those
 * not indented: a local `const` is inside a body). Each: { name, kind,
 * line, column }, counted from 0.
 */
function declarations(text) {
    const lines = blankComments(text).split("\n");
    const out = [];
    const id = "[A-Za-z_][A-Za-z0-9_]*";
    const fn = new RegExp(`^\\s*(?:export\\s+)?(?:declare\\s+)?(function|action)\\s+(${id})`);
    const ty = new RegExp(`^(?:export\\s+)?type\\s+(${id})`);
    const cst = new RegExp(`^(?:export\\s+)?const\\s+(${id})`);
    lines.forEach((l, n) => {
        let m;
        if ((m = fn.exec(l))) {
            out.push({ name: m[2], kind: m[1], line: n, column: m[0].length - m[2].length });
        } else if ((m = ty.exec(l))) {
            out.push({ name: m[1], kind: "type", line: n, column: m[0].length - m[1].length });
            // the constructors of a sum type: `= A | B(T, U) | C;`, not an
            // alias of a record, a tuple or a function
            const eq = l.indexOf("=", m[0].length);
            const rest = eq >= 0 ? l.slice(eq + 1) : "";
            if (eq >= 0 && !/^\s*[{[(]/.test(rest) && /\|/.test(rest)) {
                const alt = /(?:^|\|)\s*([A-Z][A-Za-z0-9_]*)/g;
                let a;
                while ((a = alt.exec(rest))) {
                    out.push({ name: a[1], kind: "constructor", line: n, column: eq + 1 + a.index + a[0].indexOf(a[1]) });
                }
            }
        } else if ((m = cst.exec(l))) {
            out.push({ name: m[1], kind: "const", line: n, column: m[0].length - m[1].length });
        }
    });
    return out;
}

/** A message of the Language Server Protocol: its header, then its JSON. */
function encodeMessage(msg) {
    const body = Buffer.from(JSON.stringify(msg), "utf8");
    return Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, "ascii"), body]);
}

/**
 * The messages of a stream of the protocol, given in chunks of any size:
 * `push(chunk)` calls `onMessage` with each complete one.
 */
class MessageReader {
    constructor(onMessage) {
        this.onMessage = onMessage;
        this.buf = Buffer.alloc(0);
    }
    push(chunk) {
        this.buf = Buffer.concat([this.buf, chunk]);
        for (;;) {
            const end = this.buf.indexOf("\r\n\r\n");
            if (end < 0) return;
            const m = /content-length:\s*(\d+)/i.exec(this.buf.slice(0, end).toString("ascii"));
            const len = m ? Number(m[1]) : 0;
            if (this.buf.length < end + 4 + len) return;
            const body = this.buf.slice(end + 4, end + 4 + len).toString("utf8");
            this.buf = this.buf.slice(end + 4 + len);
            let msg;
            try {
                msg = JSON.parse(body);
            } catch (e) {
                continue;
            }
            this.onMessage(msg);
        }
    }
}

/**
 * The language server: the setting `isocron.languageServer` when given,
 * else the `isc-lsp` beside the compiler when it is a path, else `isc-lsp`
 * from the PATH.
 */
function serverPath(setting, iscc, platform) {
    if (setting) return setting;
    const p = platform === "win32" ? path.win32 : path.posix;
    const exe = platform === "win32" ? "isc-lsp.exe" : "isc-lsp";
    return /[\\/]/.test(iscc || "") ? p.join(p.dirname(iscc), exe) : exe;
}

/** The modules a file imports: the texts of `from "..."` (`./geom`, `util`). */
function imports(text) {
    const out = [];
    const re = /\bimport\b[^;]*?\bfrom\s*"([^"]+)"/g;
    const t = blankComments(text);
    let m;
    while ((m = re.exec(t))) out.push(m[1]);
    return out;
}

/** The program `iscc build` makes of `file`: its name without `.isc`. */
function outputPath(file, windows) {
    const base = file.endsWith(".isc") ? file.slice(0, -4) : file + ".out";
    return windows ? base + ".exe" : base;
}

/** `setting` with `${workspaceFolder}` replaced by `folder` (when there is one). */
function expandFolder(setting, folder) {
    return folder === undefined ? setting : setting.split("${workspaceFolder}").join(folder);
}

/** The arguments of a command line: words, or texts between double quotes. */
function splitArgs(s) {
    const out = [];
    const re = /"((?:[^"\\]|\\.)*)"|(\S+)/g;
    let m;
    while ((m = re.exec(s))) out.push(m[1] !== undefined ? m[1].replace(/\\(.)/g, "$1") : m[2]);
    return out;
}

module.exports = {
    parseDiagnostics,
    blankComments,
    declarations,
    imports,
    outputPath,
    expandFolder,
    splitArgs,
    encodeMessage,
    MessageReader,
    serverPath,
};
