//! Window geometry, kept in the portable tree.
//!
//! Replaces `tauri-plugin-window-state`: that plugin writes to
//! `PathResolver::app_dir()`, which on Windows is
//! `%APPDATA%\<bundle identifier>` resolved through `SHGetKnownFolderPath` —
//! unredirectable from inside the process, and the second of the two junk
//! directories this build exists to remove.
//!
//! Persistence hangs off the global window event handler in `main` (moved and
//! resized) plus a write on quit; both go through [`save`], which is a no-op
//! unless something actually changed. A saved position is only restored when it
//! still overlaps a monitor that exists now, so a window dragged onto a monitor
//! that is later unplugged comes back on-screen instead of at coordinates that
//! no longer exist.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize};

use crate::portable::window_state_file;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    #[serde(default)]
    maximized: bool,
}

/// Serialized as `{ "main": {...}, "logs": {...} }`. `BTreeMap` for a stable
/// file: the point of this file is to be readable by whoever is debugging a
/// window that came back in the wrong place.
type State = BTreeMap<String, Geometry>;

static LAST_WRITTEN: Mutex<Option<State>> = Mutex::new(None);

fn read_state() -> State {
    std::fs::read_to_string(window_state_file())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Restore the geometry saved for each window, if any.
///
/// A saved position is only applied when it still overlaps one of the monitors
/// attached right now; otherwise the window keeps the size and position its
/// config declares, which is the only thing guaranteed to be on-screen.
pub fn restore(app: &AppHandle) {
    for (label, geometry) in read_state() {
        let Some(window) = app.get_window(&label) else {
            continue;
        };

        let position = PhysicalPosition::new(geometry.x, geometry.y);
        if position_is_on_a_monitor(app, position, &geometry) {
            let _ = window.set_position(tauri::Position::Physical(position));
        }
        let _ = window.set_size(tauri::Size::Physical(PhysicalSize::new(
            geometry.width,
            geometry.height,
        )));
        if geometry.maximized {
            let _ = window.maximize();
        }
    }
}

/// Whether `position` lands on a monitor that exists now. A window saved on a
/// since-removed monitor reports coordinates that overlap no current monitor.
fn position_is_on_a_monitor(
    app: &AppHandle,
    position: PhysicalPosition<i32>,
    geometry: &Geometry,
) -> bool {
    let Ok(monitors) = app.available_monitors() else {
        return false;
    };
    monitors.iter().any(|monitor| {
        let origin = monitor.position();
        let size = monitor.size();
        // Overlap test rather than "origin is inside": a window saved at a
        // negative offset on a monitor left of the primary is legal and
        // common, but one at `origin.x - width` is entirely off it.
        position.x + geometry.width as i32 > origin.x
            && position.x < origin.x + size.width as i32
            && position.y + geometry.height as i32 > origin.y
            && position.y < origin.y + size.height as i32
    })
}

/// Snapshot the current geometry of every window and write it if it changed.
///
/// Called from the window event handler (move/resize) and on quit; the
/// `LAST_WRITTEN` guard is what makes that cheap enough to call per event.
pub fn save(app: &AppHandle) {
    let mut state = State::new();
    for (label, window) in app.windows() {
        // A hidden window reports its last geometry, which is what we want:
        // the tray "hide the overlay" gesture must not lose its position.
        let Ok(position) = window.outer_position() else {
            continue;
        };
        let Ok(size) = window.inner_size() else {
            continue;
        };
        if size.width == 0 || size.height == 0 {
            continue;
        }
        state.insert(
            label,
            Geometry {
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
                maximized: window.is_maximized().unwrap_or(false),
            },
        );
    }

    if state.is_empty() {
        return;
    }
    {
        let last = LAST_WRITTEN.lock().unwrap();
        if last.as_ref() == Some(&state) {
            return;
        }
    }

    if let Err(e) = write(&state) {
        // Losing window geometry is not worth interrupting anyone over, but a
        // silently failing write is worth seeing when debugging.
        log::warn!(
            "could not save window state to {}: {e}",
            window_state_file().display()
        );
    }
    *LAST_WRITTEN.lock().unwrap() = Some(state);
}

fn write(state: &State) -> std::io::Result<()> {
    let path = window_state_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(state).unwrap_or_else(|_| "{}".into());
    // Write-then-rename: a kill (or a crash) mid-write must not leave a
    // truncated file that parses as "no state" on the next launch.
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path)
}

/// Forget the stored geometry, so the next launch uses the config defaults.
/// Called by the tray's "Reset Windows" so a reset survives a restart.
pub fn clear() {
    let _ = std::fs::remove_file(window_state_file());
    *LAST_WRITTEN.lock().unwrap() = Some(State::new());
}
