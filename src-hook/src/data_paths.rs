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
///
/// Looked up by name through `GetModuleHandleW` rather than by address through
/// `GetModuleHandleExW` + `GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS`: the two
/// names below are the only ones the app ever injects, an injected module is in
/// the process's module list under exactly that name, and the by-name signature
/// is the one [`crate::process`] already compiles against. The by-address call
/// needs a callback whose ABI has moved between windows-rs releases, for no
/// gain here.
///
/// `hook-dbg.dll` is the dev DLL, `hook.dll` the release one; whichever is
/// mapped wins. If neither is — a diagnostic harness loading this module under
/// some other name — the host executable's directory is still a better guess
/// than nothing.
#[cfg(windows)]
fn hook_dir() -> Option<PathBuf> {
    use windows::core::PCWSTR;
    use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};

    // Interpolated into the UTF-16 buffer below, so the names stay readable
    // instead of an array of code points.
    fn load_module(module: &str) -> Option<windows::Win32::Foundation::HMODULE> {
        let mut name: Vec<u16> = module.encode_utf16().collect();
        name.push(0);
        // `GetModuleHandleW` searches only modules already mapped into this
        // process, which is exactly the set that can hold the injected hook.
        unsafe { GetModuleHandleW(PCWSTR(name.as_ptr())) }.ok()
    }

    let module = ["hook-dbg.dll", "hook.dll"]
        .into_iter()
        .find_map(load_module);

    if let Some(module) = module {
        let mut buffer = [0u16; 512];
        let len = unsafe { GetModuleFileNameW(module, &mut buffer) } as usize;
        if len > 0 && len < buffer.len() {
            if let Some(dir) = Path::new(&String::from_utf16_lossy(&buffer[..len])).parent() {
                return Some(dir.to_path_buf());
            }
        }
    }

    // No hook module mapped (the diagnostic harnesses, a `cargo test` binary):
    // its own executable's directory is the closest thing to an answer.
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
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

    /// The module-path fallback resolves an absolute, existing directory — this
    /// test binary's own — proving the `GetModuleHandleW` + `GetModuleFileNameW`
    /// pair works rather than silently returning `None`.
    #[test]
    fn the_module_path_resolves_to_an_absolute_directory() {
        let dir = hook_dir().expect("the hook module's own directory");
        assert!(dir.is_absolute(), "{}", dir.display());
        assert!(dir.exists(), "{}", dir.display());
    }
}
