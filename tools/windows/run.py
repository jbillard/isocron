#!/usr/bin/env python3
"""Runs a program built by `iscc build --target x86_64-windows` on a Windows
machine reached by SSH (D246), as if it ran here: its standard output and
error, and its exit code, are this command's.

    run.py [--dir DIR] [--stdin FILE] [--env NAME=VALUE]... PROGRAM.exe [ARG]...

The program and the files of DIR (not its subdirectories) are copied into
a new directory of the Windows machine, where the program runs with the
arguments as they are (relative paths name the files of DIR); FILE is its
standard input. The directory is removed afterwards.

The machine: ISC_WINDOWS_SSH (user@host, default isocron@127.0.0.1),
ISC_WINDOWS_PORT (default 2222), ISC_WINDOWS_KEY (the private key, required).
A program still running after ISC_WINDOWS_TIMEOUT seconds (default 300) is
stopped, with the exit code 124.
The connections share one SSH session (ControlMaster) for 10 minutes.
"""

import os
import shlex
import subprocess
import sys
import uuid


def main():
    args = sys.argv[1:]
    dir_, stdin, env = None, None, []
    while args and args[0].startswith("--"):
        opt = args.pop(0)
        if opt == "--dir":
            dir_ = args.pop(0)
        elif opt == "--stdin":
            stdin = args.pop(0)
        elif opt == "--env":
            env.append(args.pop(0))
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
        sys.exit("run.py: cannot reach the Windows machine: " + r.stderr.decode(errors="replace"))
    try:
        r = subprocess.run(["scp", "-q", "-P", port] + ssh_opts + files + ["%s:%s/" % (host, remote)], capture_output=True)
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
        try:
            r = ssh(line, timeout=float(os.environ.get("ISC_WINDOWS_TIMEOUT", "300")))
            code = r.returncode
        except subprocess.TimeoutExpired:
            # stopped there too, so that it does not run on
            ssh("taskkill /f /im %s" % os.path.basename(exe), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            sys.stderr.write("run.py: stopped after the time allowed (ISC_WINDOWS_TIMEOUT)\n")
            code = 124
    finally:
        ssh('rmdir /s /q "%s"' % remote.replace("/", "\\"), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    sys.exit(code)


if __name__ == "__main__":
    main()
