# The test of the extension in VS Code on Windows (D267), run by windows.py
# in the directory it copied (isocron.vsix, iscc.exe, ext/, index.js,
# workspace/): the .vsix installed by the command line of VS Code and found
# in its list, then the steps of index.js in VS Code, printed. Exit code 0
# when everything passed.
#   run.ps1 -Code FOLDER_OF_VSCODE -Version VERSION
param([string]$Code, [string]$Version)
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$cli = Join-Path $Code "bin\code.cmd"

& $cli --install-extension (Join-Path $here "isocron.vsix") --force | Out-Host
$list = @(& $cli --list-extensions --show-versions)
Write-Output ("installed: " + ($list -join ", "))
if (-not ($list -contains "isocron.isocron@$Version")) {
    Write-Output "FAIL the package isocron.vsix is not installed as isocron.isocron@$Version"
    exit 1
}

$argv = @("--extensionDevelopmentPath=$here\ext", "--extensionTestsPath=$here\index.js", "$here\workspace",
    "--disable-gpu", "--skip-welcome", "--skip-release-notes", "--disable-workspace-trust", "--disable-telemetry")
$p = Start-Process -FilePath (Join-Path $Code "Code.exe") -ArgumentList $argv -PassThru
if (-not $p.WaitForExit(300000)) {
    Stop-Process -Id $p.Id -Force
    Write-Output "FAIL VS Code still running after 300 s"
    exit 1
}
$results = Join-Path $here "results.json"
if (-not (Test-Path $results)) {
    Write-Output "FAIL no results.json (VS Code exited with code $($p.ExitCode))"
    exit 1
}
$failed = 0
foreach ($r in (Get-Content $results -Raw | ConvertFrom-Json)) {
    $mark = if ($r.ok) { "ok  " } else { $failed++; "FAIL" }
    Write-Output ("$mark $($r.name): $($r.detail)")
}
Write-Output "VS Code exited with code $($p.ExitCode)"
if ($failed -gt 0 -or $p.ExitCode -ne 0) { exit 1 }
exit 0
