#!/usr/bin/env python3
"""The extension of VS Code tried in a real VS Code on Windows (D267), on
the machine of tools/windows/run.py: a portable VS Code (unpacked once from
VSCODE_ZIP into C:\\isc\\vscode-VERSION, kept there), the .vsix of
package.py installed by its command line, then the steps of
test/integration/index.js run inside it on the files of the .vsix, with
ISCC_EXE as the compiler.

    windows.py VSCODE_ZIP ISCC_EXE

VSCODE_ZIP: the "win32-x64-archive" of a version of VS Code (1.75 or later);
ISCC_EXE: an iscc.exe (of a release, or built by tools/release/build_iscc.sh).
The machine: as for run.py (ISC_WINDOWS_SSH, ISC_WINDOWS_PORT,
ISC_WINDOWS_KEY). The directory of a run is removed afterwards.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import uuid
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
EXT = os.path.dirname(HERE)


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    vscode_zip, iscc = sys.argv[1], sys.argv[2]
    key = os.environ.get("ISC_WINDOWS_KEY")
    if not key:
        sys.exit("windows.py: ISC_WINDOWS_KEY is not set")
    host = os.environ.get("ISC_WINDOWS_SSH", "isocron@127.0.0.1")
    port = os.environ.get("ISC_WINDOWS_PORT", "2222")
    control = "/tmp/isc-windows-%s-%s" % (os.getuid(), port)
    ssh_opts = ["-i", key, "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
                "-o", "LogLevel=ERROR", "-o", "ControlMaster=auto", "-o", "ControlPath=" + control, "-o", "ControlPersist=600"]

    def ssh(command, **kw):
        return subprocess.run(["ssh", "-p", port] + ssh_opts + [host, command], **kw)

    def scp(sources, target):
        r = subprocess.run(["scp", "-q", "-r", "-P", port] + ssh_opts + sources + ["%s:%s" % (host, target)], capture_output=True)
        if r.returncode != 0:
            sys.exit("windows.py: cannot copy the files: " + r.stderr.decode(errors="replace"))

    with zipfile.ZipFile(vscode_zip) as z:
        code_version = json.loads(z.read("resources/app/package.json"))["version"]
    with open(os.path.join(EXT, "package.json"), encoding="utf-8") as h:
        ext_version = json.load(h)["version"]
    if subprocess.run(["ssh", "-p", port] + ssh_opts + ["-O", "check", host], capture_output=True).returncode != 0:
        subprocess.run(["ssh", "-p", port] + ssh_opts + ["-O", "exit", host], capture_output=True)

    # VS Code, unpacked once (portable: its settings and extensions in data\)
    code = "C:\\isc\\vscode-" + code_version
    r = ssh('if exist "%s\\Code.exe" (echo yes) else (echo no)' % code, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("windows.py: cannot reach the Windows machine: " + r.stderr)
    if r.stdout.strip() != "yes":
        print("VS Code %s copied and unpacked into %s" % (code_version, code), flush=True)
        ssh('if not exist C:\\isc mkdir C:\\isc', capture_output=True)
        scp([vscode_zip], "C:/isc/vscode-%s.zip" % code_version)
        r = ssh('powershell -NoProfile -Command "Expand-Archive -LiteralPath %s.zip -DestinationPath %s -Force; '
                'New-Item -ItemType Directory -Force %s\\data | Out-Null; Remove-Item %s.zip"' % (code, code, code, code),
                capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit("windows.py: cannot unpack VS Code: " + r.stderr + r.stdout)

    with tempfile.TemporaryDirectory() as stage:
        vsix = os.path.join(stage, "isocron.vsix")
        subprocess.run([sys.executable, os.path.join(EXT, "package.py"), vsix], check=True, stdout=subprocess.DEVNULL)
        # the files the tests load are those of the package
        with zipfile.ZipFile(vsix) as z:
            for n in z.namelist():
                if n.startswith("extension/"):
                    dst = os.path.join(stage, "ext", n[len("extension/"):])
                    os.makedirs(os.path.dirname(dst), exist_ok=True)
                    with open(dst, "wb") as h:
                        h.write(z.read(n))
        shutil.copy(iscc, os.path.join(stage, "iscc.exe"))
        for f in ["index.js", "run.ps1"]:
            shutil.copy(os.path.join(HERE, "integration", f), stage)
        shutil.copytree(os.path.join(HERE, "integration", "workspace"), os.path.join(stage, "workspace"))
        remote = "C:/isc/" + uuid.uuid4().hex[:12]
        r = ssh('mkdir "%s"' % remote.replace("/", "\\"), capture_output=True)
        if r.returncode != 0:
            sys.exit("windows.py: cannot reach the Windows machine: " + r.stderr.decode(errors="replace"))
        try:
            scp([os.path.join(stage, f) for f in sorted(os.listdir(stage))], remote + "/")
            r = ssh('powershell -NoProfile -ExecutionPolicy Bypass -File "%s\\run.ps1" -Code "%s" -Version %s'
                    % (remote.replace("/", "\\"), code, ext_version), timeout=900)
            status = r.returncode
        finally:
            ssh('rmdir /s /q "%s"' % remote.replace("/", "\\"), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    sys.exit(status)


if __name__ == "__main__":
    main()
