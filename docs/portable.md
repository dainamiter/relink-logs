# Portable build

This build ships as a plain `.exe` in a folder. Everything it writes stays in
that folder — no installer, and nothing in the user profile.

## Why a launcher script could not do this

Setting `APPDATA` / `LOCALAPPDATA` / `USERPROFILE` in a `.bat` before starting
the app does nothing for the paths that mattered. Windows resolves its known
folders (`{FOLDERID_RoamingAppData}`, `{FOLDERID_LocalAppData}`) through
`SHGetKnownFolderPath`, which reads the registry and ignores those environment
variables. Tauri reaches them through `dirs-next`, wry reaches them for
WebView2, and the hook reached them through `dirs` — all of them landed in
`C:\Users\<you>` no matter what the launcher exported. That is why the fix has
to be in the source.

## Where things go now

All paths are relative to the folder containing the executable. The one knob is
`RELINK_LOGS_PORTABLE_DIR`, which moves the data root elsewhere (absolute, or
relative to the exe); leave it unset for the normal portable behaviour.

```text
GBFR Logs.exe
hook.dll
assets/
lang/
data/                       logs.db, settings.db, logs/ (the Tauri log plugin)
config/                     hook-config.json, gbfr-logs.txt (the hook's fern log)
                            and reload-debug.log in debug builds
AppData/
  Local/com.false/          WebView2 user data: cache, localStorage, cookies
  Roaming/.window-state     window geometry and positions
```

### What was moved

| Was | Now | How |
| --- | --- | --- |
| `logs.db`, `settings.db` next to the installed exe (the CWD, which a shortcut pinned to the install directory) | `data/` | `src-tauri/src/portable.rs`, `data_paths.rs` |
| `%APPDATA%\gbfr-logs\hook-config.json`, `gbfr-logs.txt` (the injected hook's readings) | `config/` | the app passes `GBFR_LOGS_DATA_DIR`; the hook falls back to its own DLL path (`src-hook/src/data_paths.rs`) |
| `%LOCALAPPDATA%\com.false` (WebView2 user data) | `AppData/Local/com.false` | `WEBVIEW2_USER_DATA_FOLDER` set in `portable::prepare()` |
| `%APPDATA%\com.false\.window-state` | `AppData/Roaming/.window-state` | `tauri-plugin-window-state` removed; `src-tauri/src/window_state.rs` replaces it |

The WebView2 folder is the awkward one: Tauri v1 hardcodes it to
`{FOLDERID_LocalAppData}\<bundle identifier>` in `WindowManager::prepare_window`,
and `tauri.conf.json` has no setting for it. The WebView2 loader documents
`WEBVIEW2_USER_DATA_FOLDER` as overriding the `userDataFolder` it is handed, so
that variable is set before the first webview is created. The CI smoke test
asserts the result rather than trusting the documentation: it launches the built
exe and fails if `com.false` or `gbfr-logs` appears in either real AppData
folder (resolved through the known-folder API, not the environment).

`tauri-plugin-window-state` was replaced rather than redirected because it
resolves `app_dir()` internally and there is no environment override for it in
Tauri v1.

## Building

`.github/workflows/portable.yaml` builds hook.dll and the app on
`windows-latest`, assembles `GBFR Logs/`, runs the smoke test, and uploads the
folder as a run artifact. It needs no secrets, so a fork can run it. Run it
from **Actions → Portable Build → Run workflow**; it also runs on pushes that
touch the source.

Locally (Windows, nightly Rust, Node):

```powershell
npm ci
npm run build
cargo build --release --package hook --features eject
Copy-Item target/release/hook.dll src-tauri/hook.dll -Force
npx tauri build --no-bundle
```

`target/release/gbfr-logs.exe` plus `hook.dll`, `assets/` and `lang/` from
`src-tauri/` is the whole portable folder.

To verify by hand on a machine that has run the installed build before: delete
(or rename) `%APPDATA%\com.false` and `%LOCALAPPDATA%\com.false`, start the
portable exe from its own folder, and confirm neither directory comes back while
`AppData\Local\com.false` and `data\logs.db` appear beside the exe.
