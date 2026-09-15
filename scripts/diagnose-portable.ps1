# Prints everything needed to judge the portable build without the job log.
#
# The smoke test answers one question ("did anything land in the profile?"), and
# when it fails this says where and why. Every line also goes to the run's step
# summary, so the report is readable from the job page with no sign-in and no
# artifact download.
#
# Read-only apart from the process probe: it launches the exe only if asked to.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $AppDir,

    [string] $ProcessName = 'GBFR Logs',

    [switch] $Launch,

    [int] $StartupSeconds = 20
)

# One sink for console and step summary, so the two cannot disagree.
function Note([string] $line) {
    Write-Host $line
    if ($env:GITHUB_STEP_SUMMARY) {
        Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Value $line -ErrorAction SilentlyContinue
    }
}

function Section([string] $title) {
    Note ''
    Note "===== $title ====="
}

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
        Roaming = '{3EB685DB-65F9-4CF6-A03A-E3EF65729F3D}'
        Local   = '{F1B32785-6FBA-4FCF-9D55-7B8E7F157091}'
    }
    return [KnownFolders]::Path($ids[$Name])
}

$roaming = Get-KnownFolder Roaming
$local = Get-KnownFolder Local

Section 'environment'
Note "cwd                        = $(Get-Location)"
Note "roaming (real known folder)= $roaming"
Note "local   (real known folder)= $local"
Note "APPDATA (this process)     = $env:APPDATA"
Note "LOCALAPPDATA (this process)= $env:LOCALAPPDATA"
Note "WEBVIEW2_USER_DATA_FOLDER  = $env:WEBVIEW2_USER_DATA_FOLDER"

Section 'WebView2 runtime'
$runtimeKeys = @(
    'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
    'HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
)
$runtimeFound = $false
foreach ($key in $runtimeKeys) {
    if (Test-Path $key) {
        $runtimeFound = $true
        Note "installed: $key"
        Note ("  version {0}" -f (Get-ItemProperty $key).pv)
    }
}
if (-not $runtimeFound) {
    Note 'NOT FOUND in the registry - a WebView2 app cannot start on this machine'
}

Section "portable folder ($AppDir)"
$resolved = Resolve-Path -LiteralPath $AppDir -ErrorAction SilentlyContinue
if (-not $resolved) {
    Note "MISSING: $AppDir"
} else {
    $root = $resolved.Path
    Note "root = $root"
    Get-ChildItem -LiteralPath $root -Force | ForEach-Object {
        $size = if ($_.PSIsContainer) { '<DIR>' } else { $_.Length }
        Note ("  {0,-10} {1}" -f $size, $_.Name)
    }

    Section 'expected paths'
    foreach ($relative in @(
            'data/logs.db',
            'config',
            'AppData/Local/com.false',
            'AppData/Roaming',
            'hook.dll',
            'assets',
            'lang')) {
        $path = Join-Path $root $relative
        $exists = Test-Path -LiteralPath $path
        Note ("  {0,-26} {1}" -f $relative, $(if ($exists) { 'present' } else { 'MISSING' }))
    }

    Section 'full tree (depth 3)'
    Get-ChildItem -LiteralPath $root -Recurse -Depth 3 -Force -ErrorAction SilentlyContinue |
        ForEach-Object { Note ("  " + $_.FullName.Substring($root.Length).TrimStart('\')) }
}

Section 'user profile leftovers'
foreach ($pair in @(@('Roaming', $roaming), @('Local', $local))) {
    $profile = $pair[1]
    foreach ($name in @('com.false', 'gbfr-logs')) {
        $path = Join-Path $profile $name
        if (Test-Path -LiteralPath $path) {
            Note "PRESENT: $path"
            Get-ChildItem -LiteralPath $path -Recurse -Depth 2 -Force -ErrorAction SilentlyContinue |
                Select-Object -First 20 |
                ForEach-Object { Note ("    " + $_.FullName.Substring($path.Length).TrimStart('\')) }
        } else {
            Note "absent:  $path"
        }
    }
}

if ($Launch) {
    Section 'launch'
    $exe = Join-Path (Resolve-Path -LiteralPath $AppDir).Path "$ProcessName.exe"
    Note "exe = $exe"
    $process = $null
    try {
        $process = Start-Process -FilePath $exe -PassThru
        Note "started pid $($process.Id)"
    } catch {
        Note "could not launch: $($_.Exception.Message)"
    }
    Start-Sleep -Seconds $StartupSeconds
    if ($process) {
        $process.Refresh()
        Note "has exited = $($process.HasExited)"
        if ($process.HasExited) { Note "exit code  = $($process.ExitCode)" }
    }
    Get-Process -Name msedgewebview2 -ErrorAction SilentlyContinue |
        Group-Object -Property Path |
        ForEach-Object { Note "webview2 process: $($_.Name) x$($_.Count)" }
    Get-Process -Name $ProcessName -ErrorAction SilentlyContinue |
        ForEach-Object { Note "app process still running: pid $($_.Id)" }
    Get-Process -Name $ProcessName, msedgewebview2 -ErrorAction SilentlyContinue |
        Stop-Process -Force -ErrorAction SilentlyContinue
}

Section 'done'
