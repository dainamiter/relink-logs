use std::path::PathBuf;

/// Returns the directory containing the currently running executable.
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        })
}

/// Returns the portable data root: `<exe_dir>/portable_data`
pub fn portable_root() -> PathBuf {
    exe_dir().join("portable_data")
}

/// Returns the WebView2 data directory: `<portable_root>/WebView2`
pub fn webview_data_dir() -> PathBuf {
    portable_root().join("WebView2")
}

/// Returns the database path: `<exe_dir>/logs.db`
pub fn db_path() -> PathBuf {
    exe_dir().join("logs.db")
}

/// Returns the logs directory: `<exe_dir>/logs`
pub fn logs_dir() -> PathBuf {
    exe_dir().join("logs")
}

/// Ensures all portable directories exist.
pub fn ensure_dirs() -> std::io::Result<()> {
    std::fs::create_dir_all(webview_data_dir())?;
    std::fs::create_dir_all(logs_dir())?;
    Ok(())
}
