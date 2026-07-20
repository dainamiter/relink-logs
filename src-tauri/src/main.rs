// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    collections::HashMap,
    fs::File,
    io::Write,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::Context;
use gbfr_logs::{db, overmastery, parser, synthesis};

use db::logs::LogEntry;
use dll_syringe::{process::OwnedProcess, Syringe};
use interprocess::os::windows::named_pipe::tokio::RecvPipeStream;
use log::{info, LevelFilter};
use parser::{
    constants::{CharacterType, EnemyType},
    v1::{self, PlayerData},
};
use protocol::Message;
use rusqlite::params_from_iter;
use serde::{Deserialize, Serialize};
use tauri::{
    api::dialog::blocking::FileDialogBuilder, AppHandle, CustomMenuItem, LogicalSize, Manager,
    Size, State, SystemTray, SystemTrayEvent, SystemTrayMenu, SystemTrayMenuItem,
};
use tauri_plugin_log::LogTarget;
use tauri_plugin_window_state::{AppHandleExt, StateFlags};
use tokio_stream::StreamExt;
use tokio_util::codec::FramedRead;

struct AlwaysOnTop(AtomicBool);
struct ClickThrough(AtomicBool);
struct DebugMode(AtomicBool);

/// Sender half of the live parser's reset channel. `None` until a parser is
/// connected; replaced on every reconnect (the parser is owned by the
/// pipe-reading task, so commands reach it through this channel).
struct ResetChannel(std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<()>>>);

#[tauri::command]
fn reset_encounter(state: State<ResetChannel>) {
    if let Some(tx) = state.0.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
}

/// Toolbox / Synthesis Helper: snapshot the game's synthesis state and report
/// whether predictions are currently possible.
#[tauri::command(async)]
async fn fetch_synthesis_status() -> Result<synthesis::SynthesisStatus, String> {
    tokio::task::spawn_blocking(|| match synthesis::snapshot::take_snapshot() {
        Ok(None) => Ok(synthesis::SynthesisStatus {
            game_running: false,
            sigil_count: 0,
            rng_unpredictable: false,
        }),
        Ok(Some(snap)) => Ok(synthesis::SynthesisStatus {
            game_running: true,
            sigil_count: snap.sigils.len() as u32,
            rng_unpredictable: snap.rng_state == 0,
        }),
        Err(e) => Err(e.to_string()),
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Toolbox / Synthesis Helper: fresh snapshot + exhaustive pair search.
#[tauri::command(async)]
async fn search_synthesis(
    query: synthesis::SynthesisQuery,
) -> Result<synthesis::SynthesisSearchResponse, String> {
    if query.trait1 == synthesis::EMPTY_TRAIT
        || query.trait2 == Some(synthesis::EMPTY_TRAIT)
    {
        return Err("invalid-trait".to_string());
    }
    tokio::task::spawn_blocking(move || {
        let snap = synthesis::snapshot::take_snapshot()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "game-not-running".to_string())?;
        let (matches, pairs_tested) = synthesis::search(&snap, &query);
        Ok(synthesis::SynthesisSearchResponse {
            matches,
            pairs_tested,
            sigil_count: snap.sigils.len() as u32,
            rng_unpredictable: snap.rng_state == 0,
            rng_state: snap.rng_state,
            seed_counter: snap.seed_counter,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Toolbox / Synthesis Helper: current seed identity for staleness polling.
/// `None` = game not running (staleness unknowable, not stale).
#[tauri::command(async)]
async fn fetch_synthesis_seed() -> Result<Option<synthesis::SynthesisSeed>, String> {
    tokio::task::spawn_blocking(|| synthesis::snapshot::take_seed_state().map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

/// Toolbox / Overmastery Predictor: is the game up, and which characters
/// exist in the roster (for the character picker).
#[tauri::command(async)]
async fn fetch_overmastery_status() -> Result<overmastery::OvermasteryStatus, String> {
    tokio::task::spawn_blocking(|| match overmastery::snapshot::take_snapshot() {
        Ok(None) => Ok(overmastery::OvermasteryStatus { game_running: false, roster: Vec::new() }),
        Ok(Some(snap)) => {
            Ok(overmastery::OvermasteryStatus { game_running: true, roster: snap.roster })
        }
        Err(e) => Err(e.to_string()),
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Toolbox / Overmastery Predictor: fresh RNG snapshot + simulate the next N
/// meditation rolls for one character and size.
#[tauri::command(async)]
async fn predict_overmastery(
    query: overmastery::OvermasteryQuery,
) -> Result<overmastery::OvermasteryPrediction, String> {
    let tables = overmastery::stock_tables();
    if query.tier >= tables.tiers.len() {
        return Err("invalid-tier".to_string());
    }
    let rolls = query.rolls.min(500);
    tokio::task::spawn_blocking(move || {
        let snap = overmastery::snapshot::take_snapshot()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "game-not-running".to_string())?;
        if snap.slot_override != u32::MAX {
            return Err("rng-override-active".to_string());
        }
        let char_idx = overmastery::char_slot_index(&snap.roster, query.char_id)
            .ok_or_else(|| "character-not-found".to_string())?;
        let slot = overmastery::rng_slot(query.tier as u32, char_idx);
        let slot_state = *snap
            .slots
            .get(slot as usize)
            .ok_or_else(|| "slot-out-of-range".to_string())?;
        Ok(overmastery::OvermasteryPrediction {
            rolls: overmastery::simulate(slot_state, query.tier, tables, rolls),
            slot,
            slot_state,
            unpredictable: slot_state == 0,
            msp_cost: tables.tiers[query.tier].msp_cost,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Toolbox / Overmastery Predictor: current RNG state of one slot, for
/// staleness polling against a prediction's `slot_state`. `None` = game not
/// running (staleness unknowable, not stale).
#[tauri::command(async)]
async fn fetch_overmastery_seed(slot: u32) -> Result<Option<u32>, String> {
    // The bound is owned by `take_slot_state`, which knows RNG_SLOT_COUNT.
    tokio::task::spawn_blocking(move || {
        overmastery::snapshot::take_slot_state(slot).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn set_debug_mode(app: AppHandle, state: State<DebugMode>, enabled: bool) {
    if let Some(window) = app.get_window("logs") {
        if enabled {
            window.open_devtools()
        } else {
            window.close_devtools()
        }
    }

    state.0.store(enabled, Ordering::Release);
}

#[tauri::command]
async fn delete_all_logs() -> Result<(), String> {
    let conn = db::connect_to_db().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM logs", [])
        .map_err(|e| e.to_string())?;
    // Deleting every log leaves every Conflux run roomless — drop them too so the
    // Conflux tab doesn't keep showing ghost "×0 rooms" runs.
    conn.execute("DELETE FROM runs", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn export_damage_log_to_file(id: u32, options: ParseOptions) -> Result<(), String> {
    let file_path = FileDialogBuilder::new()
        .add_filter("csv", &["csv"])
        .set_file_name(&format!("{id}_damage_log.csv"))
        .set_title("Export Damage Log")
        .save_file()
        .ok_or("No file selected!")?;

    let conn = db::connect_to_db().map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare("SELECT data, version FROM logs WHERE id = ?")
        .map_err(|e| e.to_string())?;

    let (blob, version): (Vec<u8>, u8) = stmt
        .query_row([id], |row| Ok((row.get(0)?, row.get(1)?)))
        .context("Failed to fetch log from database")
        .map_err(|e| e.to_string())?;

    let parser = parser::deserialize_version(&blob, version).map_err(|e| e.to_string())?;

    let file = File::create(file_path).map_err(|e| e.to_string())?;

    // @TODO(false): Split formatting into a separate function.
    let mut writer = std::io::BufWriter::new(file);

    writeln!(
        writer,
        "timestamp,source_type,child_source_type,source_index,target_type,target_index,action_id,flags,damage"
    )
    .map_err(|e| e.to_string())?;

    for (event_ts, event) in parser.encounter.event_log() {
        if let Message::DamageEvent(damage_event) = event {
            let timestamp = event_ts - parser.start_time();
            let target_type = EnemyType::from_hash(damage_event.target.parent_actor_type);
            let parent_character_type =
                CharacterType::from_hash(damage_event.source.parent_actor_type);
            let child_character_type = CharacterType::from_hash(damage_event.source.actor_type);

            if options.targets.is_empty() || options.targets.contains(&target_type) {
                writeln!(
                    writer,
                    "{},{},{},{},{},{},{},{},{}",
                    timestamp,
                    parent_character_type,
                    child_character_type,
                    damage_event.source.parent_index,
                    target_type,
                    damage_event.target.parent_index,
                    damage_event.action_id,
                    damage_event.flags,
                    damage_event.damage
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }

    writer.flush().map_err(|e| e.to_string())?;

    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResult {
    logs: Vec<LogEntry>,
    page: u32,
    page_count: u32,
    log_count: i32,
    /// IDs of the enemies that can be filtered by.
    enemy_ids: Vec<u32>,
    /// IDs of the quests that can be filtered by.
    quest_ids: Vec<u32>,
    /// Names of the Players that can be filtered by.
    player_ids: Vec<String>,
    /// Names of the Characters that can be filtered by.
    player_types: Vec<String>,
}

#[tauri::command]
fn fetch_logs(
    page: Option<u32>,
    filter_by_enemy_id: Option<u32>,
    filter_by_quest_id: Option<u32>,
    sort_direction: Option<String>,
    sort_type: Option<String>,
    quest_completed: Option<bool>,
    filter_by_player_id: Option<String>,
    filter_by_player_character: Option<String>,
) -> Result<SearchResult, String> {
    let conn = db::connect_to_db().map_err(|e| e.to_string())?;
    let page = page.unwrap_or(1);
    let per_page = 10;
    let offset = page.saturating_sub(1) * per_page;

    let sort_type_param = sort_type
        .map(|s| match s.as_str() {
            "time" => db::logs::SortType::Time,
            "duration" => db::logs::SortType::Duration,
            "quest-elapsed-time" => db::logs::SortType::QuestElapsedTime,
            _ => db::logs::SortType::Time,
        })
        .unwrap_or(db::logs::SortType::Time);

    let sort_direction_param = sort_direction
        .map(|s| match s.as_str() {
            "asc" => db::logs::SortDirection::Ascending,
            _ => db::logs::SortDirection::Descending,
        })
        .unwrap_or(db::logs::SortDirection::Descending);

    let logs = db::logs::get_logs(
        &conn,
        filter_by_enemy_id,
        filter_by_quest_id,
        per_page,
        offset,
        &sort_type_param,
        &sort_direction_param,
        quest_completed,
        &filter_by_player_id,
        &filter_by_player_character,
    )
    .map_err(|e| e.to_string())?;

    let log_count = db::logs::get_logs_count(
        &conn,
        filter_by_enemy_id,
        filter_by_quest_id,
        quest_completed,
        &filter_by_player_id,
        &filter_by_player_character,
    )
    .map_err(|e| e.to_string())?;

    let page_count = (log_count as f64 / per_page as f64).ceil() as u32;

    let mut enemy_ids = Vec::new();
    let mut quest_ids = Vec::new();
    let mut player_ids = Vec::new();
    let mut player_types = Vec::new();

    let mut query = conn
        .prepare("SELECT primary_target, quest_id, p1_name, p1_type, p2_name, p2_type, p3_name, p3_type, p4_name, p4_type from logs WHERE run_id IS NULL")
        .map_err(|e| e.to_string())?;

    let rows = query
        .query_map
