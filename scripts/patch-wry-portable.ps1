# Vendors `wry`, patches it to route the WebView2 user data folder through an
# environment variable, and leaves a `.cargo/config.toml` pointing at it.
#
#   pwsh -File scripts/patch-wry-portable.ps1 -Version 0.24.12
#
# Why this exists
# ---------------
# Tauri v1 computes the WebView2 user data folder itself, in
# `WindowManager::prepare_window`:
#
#     BaseDirectory::LocalData -> dirs_next::data_local_dir()/<bundle identifier>
#
# i.e. `%LOCALAPPDATA%\com.false`, `create_dir_all`'d, and handed to wry, which
# passes it as the explicit `userDataFolder` argument of
# `CreateCoreWebView2EnvironmentWithOptions`. An explicit argument wins over
# `WEBVIEW2_USER_DATA_FOLDER` — measured, not assumed: the portable build set
# that variable and WebView2 still created
# `C:\Users\<user>\AppData\Local\com.false\EBWebView`.
#
# `tauri.conf.json` cannot configure it in v1 (the `dataDirectory` option is a v2
# addition) and the path is resolved through `dirs-next`, which reads the
# registry rather than `LOCALAPPDATA`, so no launcher can redirect it either.
#
# The patch makes wry IGNORE the supplied directory and pass none, leaving the
# choice to `WEBVIEW2_USER_DATA_FOLDER` — which the app sets to its own folder.
# Ignoring it rather than preferring the variable also means the directory Tauri
# eagerly `create_dir_all`s never becomes the live WebView2 profile: it stays an
# empty folder whose removal is harmless, instead of one holding a browser
# profile that a later release would orphan.
#
# Nothing here is committed: this runs on the build machine into `vendor/`, which
# .gitignore excludes. The patch asserts on the exact source text, so a wry
# upgrade fails this script rather than silently shipping the unpatched layout.

[CmdletBinding()]
param(
    [string] $Version = '0.24.12',

    [string] $Root = (Split-Path -Parent $PSScriptRoot),

    # The variable the app sets to its portable WebView2 data directory. wry only
    # needs the name here; the value is decided at runtime.
    [string] $EnvVar = 'WEBVIEW2_USER_DATA_FOLDER'
)

$ErrorActionPreference = 'Stop'

$vendor = Join-Path $Root 'vendor/wry'
$target = Join-Path $vendor 'src/webview/webview2/mod.rs'

if (Test-Path $vendor) { Remove-Item $vendor -Recurse -Force }
New-Item -ItemType Directory -Force -Path (Join-Path $Root 'vendor') | Out-Null

Write-Host "==> downloading wry $Version"
$crate = Join-Path $Root "vendor/wry-$Version.crate"
Invoke-WebRequest -Uri "https://static.crates.io/crates/wry/wry-$Version.crate" -OutFile $crate -UseBasicParsing

Write-Host '==> extracting'
New-Item -ItemType Directory -Force -Path $vendor | Out-Null
tar -xzf $crate -C (Join-Path $Root 'vendor')
$extracted = Join-Path $Root "vendor/wry-$Version"
if (-not (Test-Path $extracted)) { throw "tar did not produce $extracted" }
Get-ChildItem -LiteralPath $extracted -Force | Move-Item -Destination $vendor
Remove-Item $extracted -Recurse -Force
Remove-Item $crate -Force
if (-not (Test-Path $target)) { throw "wry $Version has no src/webview/webview2/mod.rs — layout changed" }

Write-Host '==> patching'
$source = Get-Content -LiteralPath $target -Raw

$before = @'
    let data_directory = web_context
      .as_deref()
      .and_then(|context| context.data_directory())
      .and_then(|path| path.to_str())
      .map(String::from);
'@

$after = @'
    // PATCHED (relink-logs portable build): ignore the data directory Tauri v1
    // hardcodes to `%LOCALAPPDATA%\<bundle identifier>` and let WebView2 take it
    // from WEBVIEW2_USER_DATA_FOLDER, which the app points at its own folder.
    //
    // Ignoring rather than preferring the variable is deliberate: Tauri
    // `create_dir_all`s its path before this runs, so keeping the two separate
    // leaves that one an empty folder rather than a live browser profile that
    // deleting would break.
    let data_directory: Option<String> = {
      let from_env = std::env::var("__ENV_VAR__")
        .ok()
        .filter(|value| !value.is_empty());
      if from_env.is_none() {
        // No override: behave exactly as before, so a normal (installed) build
        // of this tree is unaffected.
        web_context
          .as_deref()
          .and_then(|context| context.data_directory())
          .and_then(|path| path.to_str())
          .map(String::from)
      } else {
        None
      }
    };
'@ -replace '__ENV_VAR__', $EnvVar

# Tolerate the file's CRLF/LF form without carrying a second copy of the text.
$normalise = { param($text) $text -replace "`r`n", "`n" }
$haystack = & $normalise $source
$needle = & $normalise $before
if (-not $haystack.Contains($needle)) {
    throw @'
The source does not contain the expected `let data_directory = ...` block.
wry changed; re-derive the patch from src/webview/webview2/mod.rs and update
scripts/patch-wry-portable.ps1 (do not ship wry unpatched: the portable build
depends on this).
'@
}

$patched = $haystack.Replace($needle, (& $normalise $after))
Set-Content -LiteralPath $target -Value $patched -NoNewline -Encoding utf8

# Verify by re-reading: the write above is what the build will compile.
$written = & $normalise (Get-Content -LiteralPath $target -Raw)
if (-not $written.Contains($EnvVar)) {
    throw "the patch did not survive the write to $target"
}
Write-Host ("    patched {0}" -f (Resolve-Path $target))

Write-Host '==> pointing cargo at it'
$cargoDir = Join-Path $Root '.cargo'
New-Item -ItemType Directory -Force -Path $cargoDir | Out-Null
$config = Join-Path $Root '.cargo/config.toml'
Set-Content -LiteralPath $config -Encoding utf8 -Value @"
# Generated by scripts/patch-wry-portable.ps1 — do not edit, not committed.
[patch.crates-io]
wry = { path = "vendor/wry" }
"@
Write-Host "    $config"

Write-Host ''
Write-Host "wry $Version vendored and patched; the app must set $EnvVar."
