# Smoke test for the portable build.
#
# Two things are asserted, and both matter:
#
#   1. The app really is portable — every folder it should create appears next
#      to the exe.
#   2. Nothing appears in the *real* user-profile AppData folders. This is the
#      assertion the whole change exists for, and it is checked against the
#      known-folder API rather than %APPDATA%/%LOCALAPPDATA%, because the app
#      rewrites those two environment variables for itself and would otherwise
#      be grading its own homework.
#
# The app requires elevation (manifest.xml) and shows GUI windows, so on a CI
# runner the launch is best-effort: it is allowed to fail, and the run only
# fails when a portable folder is missing or something landed in the profile.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $AppDir,

    [string] $ProcessName = 'GBFR Logs',

    [int] $StartupSeconds = 20
)

$ErrorActionPreference = 'Stop'

function Get-KnownFolder([string] $Name) {
    if (-not ('KnownFolders' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class KnownFolders {
    [DllImport("shell32.dll")]
    private static extern int SHGetKnownFolderPath(
        [MarshalAs(UnmanagedType.LPStruct)] Guid rfid, uint flags, IntPtr token, out IntPtr path);

    public static string Path(string guid) {
        IntPtr p;
        int hr = SHGetKnownFolderPath(new Guid(guid), 0, IntPtr.Zero, out p);
        if (hr != 0) { return null; }
        try { return Marshal.PtrToStringUni(p); }
        finally { Marshal.FreeCoTaskMem(p); }
    }
}
'@
    }

    $ids = @{
        Roaming = '{3EB685DB-65F9-4CF6-A03A-E3EF65729F3D}' # FOLDERID_RoamingAppData
        Local   = '{F1B32785-6FBA-4FCF-9D55-7B8E7F157091}' # FOLDERID_LocalAppData
    }
    $value = [KnownFolders]::Path($ids[$Name])
    if (-not $value) { throw "could not resolve $Name known folder" }
    return $value
}

function Test-Junk {
    param([string] $Profile, [string[]] $Names)
    $found = @()
    foreach ($name in $Names) {
        $path = Join-Path $Profile $name
        if (Test-Path -LiteralPath $path) { $found += $path }
    }
    return $found
}

# The names this app has ever used in a profile. `com.false` is the bundle
# identifier (WebView2's user data folder, and what tauri-plugin-window-state
# wrote to before it was replaced); `gbfr-logs` is the hook's old fern-log
# directory.
$junkNames = @('com.false', 'gbfr-logs')
$roaming = Get-KnownFolder Roaming
$local = Get-KnownFolder Local

$before = @(Test-Junk -Profile $roaming -Names $junkNames) +
          @(Test-Junk -Profile $local -Names $junkNames)
if ($before.Count -gt 0) {
    Write-Host "present before the launch (ignored, probably a previous run):"
    $before | ForEach-Object { Write-Host "  $_" }
    $before | ForEach-Object { Remove-Item -LiteralPath $_ -Recurse -Force }
}

$exe = Join-Path (Resolve-Path -LiteralPath $AppDir) "$ProcessName.exe"
if (-not (Test-Path -LiteralPath $exe)) { throw "no executable at $exe" }

# A clean data tree, so what appears afterwards is what this launch created.
$dataDir = Join-Path (Split-Path -Parent $exe) 'data'
$configDir = Join-Path (Split-Path -Parent $exe) 'config'
foreach ($dir in @($dataDir, $configDir, (Join-Path (Split-Path -Parent $exe) 'AppData'))) {
    if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
}

Write-Host "launching $exe"
# Best effort: the exe requires elevation and opens GUI windows. A headless
# runner can refuse the launch (no interactive desktop), and that is not what
# this script is testing — the directory assertions below still answer the
# question, one of them either way.
$process = $null
try {
    $process = Start-Process -FilePath $exe -PassThru
} catch {
    Write-Host "    could not launch: $($_.Exception.Message)"
}
# Long enough to reach WebView2 environment creation — the step that would
# write into the profile if the redirect failed — without waiting on anything
# that needs the game.
Start-Sleep -Seconds $StartupSeconds

$root = Split-Path -Parent $exe
$expected = @(
    'data/logs.db',               # the app's own database, created in main()
    'config',                     # the hook's shared directory, created in prepare()
    'AppData/Local/com.false',    # WebView2's user data folder
    'AppData/Roaming'             # window state
)
$missing = @()
foreach ($relative in $expected) {
    $path = Join-Path $root $relative
    if (Test-Path -LiteralPath $path) {
        Write-Host "  ok        $relative"
    } else {
        $missing += $relative
        Write-Host "  MISSING   $relative"
    }
}

$junk = @(Test-Junk -Profile $roaming -Names $junkNames) +
        @(Test-Junk -Profile $local -Names $junkNames)

if ($process -and -not $process.HasExited) {
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
}
Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

if ($junk.Count -gt 0) {
    Write-Host ''
    Write-Host 'FAIL: the launch wrote outside its own directory:'
    $junk | ForEach-Object { Write-Host "  $_" }
    exit 1
}

if ($missing.Count -gt 0) {
    Write-Host ''
    Write-Host 'FAIL: the portable directory tree is incomplete.'
    Write-Host 'If only AppData\Local\com.false is missing, the WebView2 user data'
    Write-Host 'redirect did not take effect (WEBVIEW2_USER_DATA_FOLDER).'
    exit 1
}

$reported = ''
if ($process -and $process.HasExited) { $reported = " (exit code $($process.ExitCode))" }
Write-Host ''
Write-Host "PASS: nothing outside $root$reported"
