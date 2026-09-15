//! Where the app keeps its writable files (logs.db, settings.db and the logs/
//! folder): [`crate::portable::data_dir`], beside the executable.
//!
//! Both platforms resolve the same portable tree. CWD-relative is not an
//! option anywhere: a desktop entry or AppImage launches with CWD at `/` (or a
//! read-only mount), an installed shortcut anchors it to the install directory,
//! and a dev run anchors it to the repo — three different data directories for
//! one app. And chdir'ing away is fatal on Linux: linuxdeploy-plugin-gtk
//! patches the bundled libwebkit2gtk's hardcoded `/usr` prefix to the
//! same-length `././`, so WebKit spawns its helper processes (e.g.
//! WebKitNetworkProcess) via paths relative to the CWD that AppRun set. The
//! process must never call `set_current_dir`.

pub use crate::portable::data_dir;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_is_absolute_and_beside_the_executable() {
        assert!(data_dir().is_absolute());
        assert!(data_dir().join("logs.db").is_absolute());
        assert_eq!(data_dir(), crate::portable::app_root().join("data"));
    }
}
