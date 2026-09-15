# Vendors `tauri` and `wry`, patches both so the app owns where its files go,
# and leaves a `.cargo/config.toml` pointing cargo at the patched copies.
#
#   pwsh -File scripts/patch-deps-portable.ps1
#
# Why this exists
# ---------------
# Two writes land in the user profile and neither can be configured away in
# Tauri v1:
#
# 1. `tauri::manager::WindowManager::prepare_window` resolves
#    `%LOCALAPPDATA%\<bundle identifier>` itself (through `dirs-next`, i.e. the
#    registry 鈥?not `APPDATA`/`LOCALAPPDATA`, so no launcher can redirect it),
#    assigns it to `webview_attributes.data_directory`, and then
#    `create_dir_all`s it. That empty `%LOCALAPPDATA%\com.false` is the residue.
# 2. wry then passes that value as the explicit `userDataFolder` argument of
#    `CreateCoreWebView2EnvironmentWithOptions`, and an explicit argument wins
#    over `WEBVIEW2_USER_DATA_FOLDER`. Measured, not assumed: setting only the
#    variable still produced `%LOCALAPPDATA%\com.false\EBWebView`.
#
# The `dataDirectory` config option that would fix (1) is a v2 addition, and (2)
# has no setting at all. So both crates are vendored and patched:
#
#   tauri  鈥?when `RELINK_LOGS_APP_DIR` is set, leave `data_directory` as None,
#            so nothing is resolved, assigned or created.
#   wry    鈥?ignore a supplied data directory when `WEBVIEW2_USER_DATA_FOLDER`
#            is set, and pass none, so the loader takes it from the variable.
#
# With both in place the app's `portable::prepare` (which sets that variable
# before the Tauri builder runs) decides the location and `%LOCALAPPDATA%` is
# never touched at all.
#
# Nothing here is committed: this runs on the build machine into `vendor/`, which
# .gitignore excludes. Every patch asserts on the exact source text, so a
# dependency bump fails this script loudly rather than silently shipping a build
# that writes to the profile again.

[CmdletBinding()]
param(
    [string] $Root = (Split-Path -Parent $PSScriptRoot),

    # The WebView2 data folder the app sets in `portable::prepare`.
    [string] $WebViewDirEnvVar = 'WEBVIEW2_USER_DATA_FOLDER',

    # Signals "this is the portable build"; when set, tauri must not touch the
    # profile. The app sets it alongside the variable above.
    [string] $AppDirEnvVar = 'RELINK_LOGS_APP_DIR'
)

$ErrorActionPreference = 'Stop'

$vendorRoot = Join-Path $Root 'vendor'
if (Test-Path $vendorRoot) { Remove-Item $vendorRoot -Recurse -Force }
New-Item -ItemType Directory -Force -Path $vendorRoot | Out-Null

# Line endings must not matter to the literal replacements below, and the result
# must be a single string: `(Get-Content) -replace ...` yields an ARRAY, which
# `[string]::Join` then stringifies as "System.Object[]" 鈥?that silently broke
# every `.Contains()` assertion here once already.
function Get-NormalisedText([string] $text) {
    if ($null -eq $text) { return '' }
    return $text.Replace("`r`n", "`n").Replace("`r", "`n")
}

function Get-Normalised([string] $path) {
    return Get-NormalisedText ([System.IO.File]::ReadAllText($path))
}

# One sink for console and step summary: when this runs in CI, what it did has to
# be readable without the job log.
function Note([string] $line) {
    Write-Host $line
    if ($env:GITHUB_STEP_SUMMARY) {
        Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Value $line -ErrorAction SilentlyContinue
    }
}

$global:PatchReport = @()

<#
.SYNOPSIS
  Download a crate, patch one literal block in one file, verify, and return the
  vendored directory.
#>
function Invoke-VendoredPatch {
    param(
        [Parameter(Mandatory)] [string] $Name,
        [Parameter(Mandatory)] [string] $Version,
        [Parameter(Mandatory)] [string] $RelativeFile,
        [Parameter(Mandatory)] [string] $Before,
        [Parameter(Mandatory)] [string] $After
    )

    Note "==> $Name $Version"
    $crate = Join-Path $vendorRoot "$Name-$Version.crate"
    $url = "https://static.crates.io/crates/$Name/$Name-$Version.crate"
    Invoke-WebRequest -Uri $url -OutFile $crate -UseBasicParsing
    $bytes = (Get-Item -LiteralPath $crate).Length
    Note ("    downloaded {0:N0} bytes" -f $bytes)
    if ($bytes -lt 1024) { throw "$url returned $bytes bytes; not a crate" }

    Note '    extracting'
    # A `.crate` is a gzipped tarball; tar ships with Windows and on the runners.
    tar -xzf $crate -C $vendorRoot
    if ($LASTEXITCODE -ne 0) { throw "tar exited $LASTEXITCODE for $crate" }
    $extracted = Join-Path $vendorRoot "$Name-$Version"
    if (-not (Test-Path $extracted)) { throw "tar did not produce $extracted" }
    $target = Join-Path $vendorRoot $Name
    Get-ChildItem -LiteralPath $extracted -Force | Move-Item -Destination $target
    Remove-Item $extracted -Recurse -Force
    Remove-Item $crate -Force
    $fileCount = (Get-ChildItem -LiteralPath $target -Recurse -File -Force).Count
    Note "    extracted $fileCount files to vendor/$Name"

    $file = Join-Path $target $RelativeFile
    if (-not (Test-Path $file)) {
        throw "$Name $Version has no $RelativeFile 鈥?layout changed, re-derive the patch"
    }

    Note "    patching $RelativeFile"
    $haystack = Get-Normalised $file
    $needle = Get-NormalisedText $Before
    if (-not $haystack.Contains($needle)) {
        throw @"
${Name} ${Version} does not contain the expected block in ${RelativeFile}:

$needle

Re-derive the patch from the real source and update this script. Do NOT ship
${Name} unpatched: the portable build writes into the user profile without it.
"@
    }
    $patched = $haystack.Replace($needle, (Get-NormalisedText $After))
    Set-Content -LiteralPath $file -Value $patched -NoNewline -Encoding utf8

    # Re-read what will actually be compiled: a failed write must not pass.
    if (-not (Get-Normalised $file).Contains('PATCHED (relink-logs portable build)')) {
        throw "the $Name patch did not survive the write to $file"
    }
    Note '    patch applied and re-read'
    $global:PatchReport += "$Name $Version patched ($fileCount files)"
    return $target
}

# --- tauri: stop it creating %LOCALAPPDATA%\<identifier> ---------------------
$tauriBefore = @'
    // in `Windows`, we need to force a data_directory
    // but we do respect user-specification
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if pending.webview_attributes.data_directory.is_none() {
      let local_app_data = resolve_path(
        &self.inner.config,
        &self.inner.package_info,
        self.inner.state.get::<crate::Env>().inner(),
        &self.inner.config.tauri.bundle.identifier,
        Some(BaseDirectory::LocalData),
      );
      if let Ok(user_data_dir) = local_app_data {
        pending.webview_attributes.data_directory = Some(user_data_dir);
      }
    }
'@

$tauriAfter = @'
    // PATCHED (relink-logs portable build): when the app names its own
    // directory, do not resolve, assign or create `%LOCALAPPDATA%\<identifier>`.
    // Leaving `data_directory` as None also lets wry leave the choice to
    // WEBVIEW2_USER_DATA_FOLDER, which the app points at its own folder.
    //
    // Unset (a normal build of this tree) keeps the original behaviour.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let portable_app_dir = std::env::var("__APP_DIR_ENV__")
      .ok()
      .filter(|value| !value.is_empty())
      .is_some();

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if !portable_app_dir && pending.webview_attributes.data_directory.is_none() {
      let local_app_data = resolve_path(
        &self.inner.config,
        &self.inner.package_info,
        self.inner.state.get::<crate::Env>().inner(),
        &self.inner.config.tauri.bundle.identifier,
        Some(BaseDirectory::LocalData),
      );
      if let Ok(user_data_dir) = local_app_data {
        pending.webview_attributes.data_directory = Some(user_data_dir);
      }
    }
'@ -replace '__APP_DIR_ENV__', $AppDirEnvVar

# --- wry: let WEBVIEW2_USER_DATA_FOLDER decide ------------------------------
$wryBefore = @'
    let data_directory = web_context
      .as_deref()
      .and_then(|context| context.data_directory())
      .and_then(|path| path.to_str())
      .map(String::from);
'@

$wryAfter = @'
    // PATCHED (relink-logs portable build): when the app set the WebView2 data
    // folder itself, pass none and let the loader take it from that variable 鈥?    // an explicit `userDataFolder` argument otherwise wins over it.
    let data_directory: Option<String> = {
      let from_env = std::env::var("__WEBVIEW_ENV__")
        .ok()
        .filter(|value| !value.is_empty());
      if from_env.is_some() {
        None
      } else {
        web_context
          .as_deref()
          .and_then(|context| context.data_directory())
          .and_then(|path| path.to_str())
          .map(String::from)
      }
    };
'@ -replace '__WEBVIEW_ENV__', $WebViewDirEnvVar

Invoke-VendoredPatch -Name 'tauri' -Version '1.8.3' `
    -RelativeFile 'src/manager.rs' -Before $tauriBefore -After $tauriAfter | Out-Null

Invoke-VendoredPatch -Name 'wry' -Version '0.24.12' `
    -RelativeFile 'src/webview/webview2/mod.rs' -Before $wryBefore -After $wryAfter | Out-Null

Note '==> pointing cargo at the patched crates'
$cargoDir = Join-Path $Root '.cargo'
New-Item -ItemType Directory -Force -Path $cargoDir | Out-Null
$config = Join-Path $cargoDir 'config.toml'
Set-Content -LiteralPath $config -Encoding utf8 -Value @'
# Generated by scripts/patch-deps-portable.ps1 鈥?do not edit, not committed.
[patch.crates-io]
tauri = { path = "vendor/tauri" }
wry = { path = "vendor/wry" }
'@
Note "    $config"

Note ''
Note 'Vendored and patched. The app must set:'
Note "  $AppDirEnvVar  (checked by the patched tauri)"
Note "  $WebViewDirEnvVar  (checked by the patched wry)"
