//! Portable layout: every file the app writes lives beside the executable.
//!
//! Windows resolves its known folders (`{FOLDERID_RoamingAppData}`,
//! `{FOLDERID_LocalAppData}`) with `SHGetKnownFolderPath`, which reads the
//! registry — *not* the `APPDATA`/`LOCALAPPDATA` environment variables a
//! launcher script could set. Every dependency underneath this app (tauri's
//! `PathResolver` via `dirs-next`, wry's WebView2 data directory) resolves
//! those folders, so redirecting them from a `.bat` was never going to work.
//! The only reliable answer is to stop asking the OS where to write and keep
//! everything next to the exe.
//!
//! Layout, all under [`app_root`]:
//!
//! ```text
//! <exe dir>/
//!   GBFR Logs.exe
//!   hook.dll            resources the bundler ships flat
//!   assets/ lang/
//!   data/               logs.db, settings.db, logs/ (the tauri log plugin)
//!   config/             hook-config.json, gbfr-logs.txt (the hook's fern log)
//!   AppData/            whatever still insists on an AppData path:
//!     Local/com.false/    WebView2 user data (cache, localStorage, …)
//!     Roaming/com.false/  window state
//! ```
//!
//! `AppData/` is not a whim: wry appends the bundle identifier to the local
//! app-data directory it is handed, and it expects that directory to exist, so
//! [`prepare`] points the environment at `AppData/Local` and `AppData/Roaming`
//! here instead of at the user profile.
//!
//! One knob: `RELINK_LOGS_PORTABLE_DIR` overrides the root, for keeping data on
//! another drive while the exe stays put. A relative value resolves against the
//! exe directory. Unset means "beside the exe", the mode this build exists for.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Overrides the portable root (default: the executable's own directory).
pub const ROOT_ENV: &str = "RELINK_LOGS_PORTABLE_DIR";

/// Bundle identifier, mirroring `tauri.conf.json`. wry appends it to the local
/// app-data directory, which is where `AppData/Local/com.false` comes from.
pub const BUNDLE_IDENTIFIER: &str = "com.false";

static ROOT: OnceLock<PathBuf> = OnceLock::new();
static DATA: OnceLock<PathBuf> = OnceLock::new();
static CONFIG: OnceLock<PathBuf> = OnceLock::new();
static HOOK_LOG: OnceLock<PathBuf> = OnceLock::new();
static WINDOW_STATE: OnceLock<PathBuf> = OnceLock::new();
static WEBVIEW_DATA: OnceLock<PathBuf> = OnceLock::new();

/// The directory the executable lives in, or the process CWD if `current_exe`
/// fails (it does not in practice; the fallback only avoids a panic).
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Root of every writable file: the exe directory unless
/// [`ROOT_ENV`] says otherwise.
pub fn app_root() -> &'static Path {
    ROOT.get_or_init(|| match std::env::var_os(ROOT_ENV) {
        Some(value) if !value.is_empty() => {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                exe_dir().join(path)
            }
        }
        _ => exe_dir(),
    })
}

/// Databases (`logs.db`, `settings.db`) and the log plugin's `logs/` folder.
pub fn data_dir() -> &'static Path {
    DATA.get_or_init(|| app_root().join("data"))
}

/// Configuration the injected hook shares with the app.
pub fn config_dir() -> &'static Path {
    CONFIG.get_or_init(|| app_root().join("config"))
}

/// `hook-config.json`, read by the hook once per injection.
pub fn hook_config_file() -> PathBuf {
    config_dir().join("hook-config.json")
}

/// The fern log the injected hook appends to.
pub fn hook_log_file() -> &'static Path {
    HOOK_LOG.get_or_init(|| config_dir().join("gbfr-logs.txt"))
}

/// Window geometry, restored on the next launch.
pub fn window_state_file() -> &'static Path {
    WINDOW_STATE.get_or_init(|| app_root().join("AppData").join("Roaming").join(".window-state"))
}

/// WebView2's user data root; wry appends [`BUNDLE_IDENTIFIER`], giving
/// `AppData/Local/com.false`.
pub fn webview_data_dir() -> &'static Path {
    WEBVIEW_DATA.get_or_init(|| app_root().join("AppData").join("Local"))
}

/// Redirect every AppData lookup to the portable tree and create the
/// directories. Called first thing in `main`, before the Tauri builder and
/// therefore before any webview exists.
pub fn prepare() {
    let roaming = app_root().join("AppData").join("Roaming");
    let local = webview_data_dir();

    std::env::set_var("APPDATA", env_path(roaming.clone()));
    std::env::set_var("LOCALAPPDATA", env_path(local.clone()));

    // Tauri v1 hardcodes the WebView2 user data folder to
    // `{FOLDERID_LocalAppData}\<bundle identifier>` in
    // `WindowManager::prepare_window` — `tauri.conf.json` cannot configure it —
    // and the WebView2 loader applies this variable as an override of the
    // explicit folder it is handed. There is no in-app API for it, which is why
    // this has to be an environment variable rather than an argument.
    std::env::set_var(
        "WEBVIEW2_USER_DATA_FOLDER",
        env_path(local.join(BUNDLE_IDENTIFIER)),
    );

    // How the injected hook finds this tree. It runs inside the game process,
    // where `current_exe()` is the game, so it cannot derive the path itself;
    // the value is inherited by any game the app launches.
    std::env::set_var("GBFR_LOGS_DATA_DIR", env_path(app_root().to_path_buf()));

    // Create eagerly: a read-only exe directory should fail here, naming the
    // directory, rather than as an opaque SQLite or WebView2 error once the UI
    // is half up.
    for dir in [data_dir(), config_dir(), local.as_path(), roaming.as_path()] {
        create(dir);
    }
}

/// `create_dir_all` with the path in the panic message. A failure here is
/// unrecoverable — there is no second place the app is allowed to write.
fn create(dir: &Path) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        panic!(
            "portable data directory {} is not writable: {e}\n\
             Put the app in a writable folder, or set {ROOT_ENV} to one.",
            dir.display()
        );
    }
}

/// Forward-slashed form of an absolute path, for the environment variables
/// above. Windows accepts either separator, and this avoids the trailing-
/// backslash and `\\?\` escaping traps in values other tools re-read.
#[cfg(windows)]
fn env_path(path: PathBuf) -> String {
    let text = path.to_string_lossy();
    // \\?\C:\dir -> C:/dir ; network (\\?\UNC\) and anything else stays as-is.
    let text = match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => rest.to_string(),
        _ => text.into_owned(),
    };
    text.replace('\\', "/")
}

#[cfg(not(windows))]
fn env_path(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Absolute always: a CWD-relative data dir is the bug this module exists
    /// to remove (the installed app's shortcut anchored the CWD to the install
    /// directory, and a dev run anchored it to the repo).
    #[test]
    fn root_is_absolute() {
        assert!(app_root().is_absolute(), "{:?}", app_root());
    }

    /// Every writable path is under the root, and none of them is a user
    /// profile directory.
    #[test]
    fn writable_paths_stay_under_the_root() {
        let root = app_root();
        for path in [
            data_dir(),
            config_dir(),
            hook_log_file(),
            window_state_file(),
            webview_data_dir(),
        ] {
            assert!(
                path.starts_with(root),
                "{} escapes {}",
                path.display(),
                root.display()
            );
        }
    }

    /// The two AppData directories must sit in the portable tree, not in the
    /// user profile: that is the whole point of the module. Tauri appends the
    /// bundle identifier to the local one for WebView2, hence the extra check
    /// that it lands where the identifier is expected.
    #[test]
    fn appdata_redirects_into_the_portable_tree() {
        let expected = app_root().join("AppData");
        assert_eq!(webview_data_dir(), expected.join("Local"));
        assert_eq!(
            webview_data_dir().join(BUNDLE_IDENTIFIER),
            expected.join("Local").join("com.false")
        );
        assert_eq!(window_state_file().parent().unwrap(), expected.join("Roaming"));
        assert_eq!(hook_config_file(), config_dir().join("hook-config.json"));
    }

    /// The values handed to `APPDATA`, `LOCALAPPDATA` and
    /// `WEBVIEW2_USER_DATA_FOLDER` must not end in a backslash: those get
    /// re-serialised (JSON, `.lnk` files, `cmd`), where `"C:\dir\"` is a
    /// mangled escape rather than a path.
    #[cfg(windows)]
    #[test]
    fn env_paths_use_forward_slashes() {
        let text = env_path(PathBuf::from(r"C:\games\relink-logs\data"));
        assert_eq!(text, "C:/games/relink-logs/data");
        assert!(!text.ends_with('\\'));

        // A verbatim prefix is an implementation detail of current_exe(); it
        // must not leak into an environment variable other tools read.
        let verbatim = env_path(PathBuf::from(r"\\?\C:\games\relink-logs"));
        assert_eq!(verbatim, "C:/games/relink-logs");
    }
}
