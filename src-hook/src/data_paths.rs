//! Where the injected hook reads and writes its files.
//!
//! The hook used to call `dirs::data_dir()`, which on Windows is
//! `{FOLDERID_RoamingAppData}` straight from the registry — the reason a
//! launcher script's `set APPDATA=...` had no effect and every session left a
//! `%APPDATA%\gbfr-logs` directory behind. Nothing in the injected process may
//! touch a user profile path, so both the hook's log and the config file it
//! reads live in the app's portable tree instead.
//!
//! Two ways to find that tree, in order:
//!
//! 1. `GBFR_LOGS_DATA_DIR`, the app's portable root, which the app sets for its
//!    own process. It is inherited by a game the app launches, and the hook is
//!    injected into that game — so this is the exact directory whenever it is
//!    available.
//! 2. The hook DLL's own directory, which is the app's folder (the copy the app
//!    injects). `std::env::current_exe()` cannot be used for this: that is the
//!    *game* executable, not the module doing the writing.
//!
//! Both are best-effort. A hook that cannot find anywhere to log must still
//! inject and still parse damage — the log is a diagnostic, not a feature — so
//! every failure here degrades to "no log file" rather than to a panic.

use std::path::{Path, PathBuf};

/// The app's portable `config/` directory, or `None` if neither source
/// resolves.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = env_config_dir(std::env::var_os("GBFR_LOGS_DATA_DIR")) {
        return Some(dir);
    }
    hook_dir().map(|dir| dir.join("config"))
}

/// `hook-config.json`, where the app publishes the dev toggles the hook reads
/// once per injection.
pub fn hook_config_file() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("hook-config.json"))
}

/// The fern log. The logger creates the directory; nothing here touches disk.
pub fn log_file() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("gbfr-logs.txt"))
}

/// The `GBFR_LOGS_DATA_DIR` branch, split out so a test can exercise it without
/// mutating the process environment (which other tests read concurrently).
fn env_config_dir(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let value = value?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value).join("config"))
}

/// Directory the loaded hook module lives in.
#[cfg(windows)]
fn hook_dir() -> Option<PathBuf> {
    use windows::Win32::Foundation::{HMODULE, LPARAM};
    use windows::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };

    /// Writes the module handle into the `Option<HMODULE>` behind `lparam` —
    /// the documented alternative to passing the address of a `MaybeUninit`.
    unsafe extern "system" fn capture(module: HMODULE, lparam: LPARAM) -> windows::core::BOOL {
        *(lparam.0 as *mut Option<HMODULE>) = Some(module);
        windows::core::BOOL(1)
    }

    let mut module: Option<HMODULE> = None;
    // From an address inside this module, so it finds the hook whatever name it
    // was loaded under (`hook.dll`, `hook-dbg.dll`, a staging copy). The
    // UNCHANGED_REFCOUNT flag means the handle is borrowed, not a reference to
    // release.
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            windows::core::PCWSTR(hook_dir as *const () as *const u16),
            Some(&mut module),
        )
        .ok()?;
    }
    let module = module?;

    let mut buffer = [0u16; 512];
    let len = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
    if len == 0 || len >= buffer.len() {
        return None;
    }

    Path::new(&String::from_utf16_lossy(&buffer[..len]))
        .parent()
        .map(Path::to_path_buf)
}

/// Proton: the app deploys the hook into the *game* directory as `dinput8.dll`,
/// so the app's own directory is not reachable from here. `current_exe()` is
/// the game — the hook's log lands beside it, which is at least inside the Wine
/// prefix the user chose rather than in a user profile.
#[cfg(not(windows))]
fn hook_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    /// An empty value must not win over the module-path fallback, or a launcher
    /// that exports the variable without setting it disables logging entirely.
    #[test]
    fn an_empty_env_var_is_ignored() {
        assert_eq!(env_config_dir(Some(OsString::new())), None);
        assert_eq!(env_config_dir(None), None);
    }

    #[test]
    fn a_set_env_var_names_the_config_directory() {
        assert_eq!(
            env_config_dir(Some(OsString::from(r"D:\portable\relink-logs"))),
            Some(PathBuf::from(r"D:\portable\relink-logs").join("config"))
        );
    }

    /// The module-path fallback resolves this test binary's directory, which is
    /// a real absolute path — proof that the `GetModuleHandleExW` dance works
    /// rather than silently returning `None`.
    #[test]
    fn the_module_path_resolves_to_an_absolute_directory() {
        let dir = hook_dir().expect("the hook module's own directory");
        assert!(dir.is_absolute(), "{}", dir.display());
        assert!(dir.exists(), "{}", dir.display());
    }
}
