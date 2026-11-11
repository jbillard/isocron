#!/usr/bin/env python3
"""Runs a program built by `iscc build --target x86_64-windows` on a Windows
machine reached by SSH (D246), as if it ran here: its standard output and
error, and its exit code, are this command's.

    run.py [--dir DIR] [--stdin FILE] [--env NAME=VALUE]... [--interactive] PROGRAM.exe [ARG]...

The program and the files of DIR (not its subdirectories) are copied into
a new directory of the Windows machine, where the program runs with the
arguments as they are (relative paths name the files of DIR); FILE is its
standard input. The directory is removed afterwards.

The machine: ISC_WINDOWS_SSH (user@host, default isocron@127.0.0.1),
ISC_WINDOWS_PORT (default 2222), ISC_WINDOWS_KEY (the private key, required).
A program still running after ISC_WINDOWS_TIMEOUT seconds (default 300) is
stopped, with the exit code 124.

--interactive runs the program in the session open on the screen of the
machine (a scheduled task `/it`, D636), not in the session of SSH, which
has no desktop shown: what a window draws is there to capture. Its
standard input is then empty, its outputs read back from files (the
standard error after the output).
The connections share one SSH session (ControlMaster) for 10 minutes.
"""

import os
import shlex
import subprocess
import sys
import uuid


def main():
    args = sys.argv[1:]
    dir_, stdin, env, interactive = None, None, [], False
    while args and args[0].startswith("--"):
        opt = args.pop(0)
        if opt == "--dir":
            dir_ = args.pop(0)
        elif opt == "--stdin":
            stdin = args.pop(0)
        elif opt == "--env":
            env.append(args.pop(0))
        elif opt == "--interactive":
            interactive = True
        else:
            sys.exit("run.py: unknown option " + opt)
    if not args:
        sys.exit(__doc__)
    exe, prog_args = args[0], args[1:]
    key = os.environ.get("ISC_WINDOWS_KEY")
    if not key:
        sys.exit("run.py: ISC_WINDOWS_KEY is not set")
    host = os.environ.get("ISC_WINDOWS_SSH", "isocron@127.0.0.1")
    port = os.environ.get("ISC_WINDOWS_PORT", "2222")
    control = "/tmp/isc-windows-%s-%s" % (os.getuid(), port)
    ssh_opts = ["-i", key, "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
                "-o", "LogLevel=ERROR", "-o", "ControlMaster=auto", "-o", "ControlPath=" + control, "-o", "ControlPersist=600"]
    remote = "C:/isc/" + uuid.uuid4().hex[:12]
    files = [exe]
    if dir_:
        files += [os.path.join(dir_, f) for f in sorted(os.listdir(dir_)) if os.path.isfile(os.path.join(dir_, f))]
    if stdin and os.path.abspath(os.path.dirname(stdin)) != os.path.abspath(dir_ or "/nonexistent"):
        files.append(stdin)

    def ssh(command, **kw):
        return subprocess.run(["ssh", "-p", port] + ssh_opts + [host, command], **kw)

    # a shared session whose connection died (the machine restarted) is
    # closed first: its errors would mix with the program's (D250)
    if subprocess.run(["ssh", "-p", port] + ssh_opts + ["-O", "check", host], capture_output=True).returncode != 0:
        subprocess.run(["ssh", "-p", port] + ssh_opts + ["-O", "exit", host], capture_output=True)
    # the commands around the program keep their messages, shown on failure
    r = ssh('mkdir "%s"' % remote.replace("/", "\\"), capture_output=True)
    if r.returncode != 0:
        # once more after the shared session closed: many tests start at
        # once on a machine just started, and the first sessions may be
        # refused (D547)
        subprocess.run(["ssh", "-p", port] + ssh_opts + ["-O", "exit", host], capture_output=True)
        r = ssh('mkdir "%s"' % remote.replace("/", "\\"), capture_output=True)
    if r.returncode != 0:
        sys.exit("run.py: cannot reach the Windows machine: " + r.stderr.decode(errors="replace"))
    if interactive:
        # the script of the scheduled task: the program, its outputs to
        # files, its exit code last (what `run_interactive` waits for)
        import tempfile
        line = "cd /d %s && " % remote.replace("/", "\\")
        for e in env:
            line += 'set "%s" && ' % e
        line += os.path.basename(exe)
        for a in prog_args:
            line += ' "%s"' % a.replace('"', '\\"') if (" " in a or '"' in a or a == "") else " " + a
        script_dir = tempfile.mkdtemp()
        script = os.path.join(script_dir, "run.cmd")
        with open(script, "w", newline="\r\n") as f:
            f.write("@echo off\n%s < nul > out.txt 2> err.txt\necho %%errorlevel%% > code.txt\n" % line)
        files.append(script)
    try:
        r = subprocess.run(["scp", "-q", "-P", port] + ssh_opts + files + ["%s:%s/" % (host, remote)], capture_output=True)
        if interactive:
            os.remove(script)
            os.rmdir(script_dir)
        if r.returncode != 0:
            sys.exit("run.py: cannot copy the files: " + r.stderr.decode(errors="replace"))
        # cmd.exe: the variables, then the program with its arguments quoted
        # as the C library of the program parses them
        line = "cd /d %s && " % remote.replace("/", "\\")
        for e in env:
            line += 'set "%s" && ' % e
        line += os.path.basename(exe)
        for a in prog_args:
            line += ' "%s"' % a.replace('"', '\\"') if (" " in a or '"' in a or a == "") else " " + a
        if stdin:
            line += " < " + os.path.basename(stdin)
        if interactive:
            sys.exit(run_interactive(ssh, remote, exe))
        try:
            # the messages of ssh itself left out of the program's error: a
            # shared session that refuses one more (the sessions of a
            # machine of Windows are limited) is followed by a connection
            # of its own, whose warnings would mix with the program's (D542)
            # (the first value of an option is the one ssh keeps)
            r = subprocess.run(["ssh", "-p", port, "-o", "LogLevel=QUIET"] + ssh_opts + [host, line], timeout=float(os.environ.get("ISC_WINDOWS_TIMEOUT", "300")))
            code = r.returncode
        except subprocess.TimeoutExpired:
            # stopped there too, so that it does not run on
            ssh("taskkill /f /im %s" % os.path.basename(exe), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            sys.stderr.write("run.py: stopped after the time allowed (ISC_WINDOWS_TIMEOUT)\n")
            code = 124
    finally:
        ssh('rmdir /s /q "%s"' % remote.replace("/", "\\"), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    sys.exit(code)


def run_interactive(ssh, remote, exe):
    """The script `run.cmd` of `remote` (copied with the files) run in the
    session on the screen by a scheduled task, waited for: its output and
    error (files written by the script) and its exit code given as this
    command's."""
    import time
    win = remote.replace("/", "\\")
    task = "isc-" + os.path.basename(remote)
    ssh('schtasks /create /tn %s /tr "%s\\run.cmd" /sc once /st 23:59 /it /f' % (task, win), capture_output=True)
    ssh("schtasks /run /tn %s" % task, capture_output=True)
    limit = float(os.environ.get("ISC_WINDOWS_TIMEOUT", "300"))
    start = time.monotonic()
    code = 124
    while time.monotonic() - start < limit:
        r = ssh('type "%s\\code.txt"' % win, capture_output=True)
        if r.returncode == 0 and r.stdout.strip():
            code = int(r.stdout.strip())
            break
        time.sleep(0.5)
    if code == 124:
        ssh("taskkill /f /im %s" % os.path.basename(exe), capture_output=True)
        sys.stderr.write("run.py: stopped after the time allowed (ISC_WINDOWS_TIMEOUT)\n")
    sys.stdout.buffer.write(ssh('type "%s\\out.txt"' % win, capture_output=True).stdout)
    sys.stderr.buffer.write(ssh('type "%s\\err.txt"' % win, capture_output=True).stdout)
    ssh("schtasks /delete /tn %s /f" % task, capture_output=True)
    return code


if __name__ == "__main__":
    main()
