use std::{
    collections::HashMap,
    ffi::{c_void, CStr, CString},
    sync::{Mutex, OnceLock},
};

use anyhow::{anyhow, Result};
use protocol::{Message, PlayerIdentityEvent};
use retour::static_detour;
use windows::Win32::{Foundation::HANDLE, System::Diagnostics::Debug::ReadProcessMemory};

use crate::{
    event,
    hooks::{
        actor_type_id,
        ffi::{Overmasteries, PlayerStats, SigilList, VBuffer, WeaponInfo},
        globals::{OVERMASTERY_OFFSET, PLAYER_DATA_OFFSET, SIGIL_OFFSET, WEAPON_OFFSET},
    },
    process::Process,
};

type OnLoadPlayerFunc = unsafe extern "system" fn(*const usize) -> usize;
type RefreshPlayerIdentityFunc = unsafe extern "system" fn(*const usize);

static_detour! {
    static OnLoadPlayer: unsafe extern "system" fn(*const usize) -> usize;
    static RefreshPlayerIdentity: unsafe extern "system" fn(*const usize);
}

#[derive(Clone)]
pub struct OnLoadPlayerHook {
    tx: event::Tx,
}

impl OnLoadPlayerHook {
    pub fn new(tx: event::Tx) -> Self {
        OnLoadPlayerHook { tx }
    }

    pub fn setup(&self, process: &Process) -> Result<()> {
        let cloned_self = self.clone();

        if let Ok(on_load_player_original) =
            process.search_address("49 89 ce e8 $ { ' } 31 ff 85 c0 ? ? ? ? ? ? 49 8b 46 28")
        {
            #[cfg(feature = "console")]
            println!("Found on load player");

            unsafe {
                let func: OnLoadPlayerFunc = std::mem::transmute(on_load_player_original);
                OnLoadPlayer.initialize(func, move |a1| cloned_self.run(a1))?;
                OnLoadPlayer.enable()?;
            }
        } else {
            return Err(anyhow!("Could not find on_load_player"));
        }

        Ok(())
    }

    fn run(&self, a1: *const usize) -> usize {
        #[cfg(feature = "console")]
        println!("on load player: {:p}", a1);

        let ret = unsafe { OnLoadPlayer.call(a1) };

        let player_idx = unsafe { a1.byte_add(0x170).read() } as u32;

        let player_offset = PLAYER_DATA_OFFSET.load(std::sync::atomic::Ordering::Relaxed);
        let weapon_offset = WEAPON_OFFSET.load(std::sync::atomic::Ordering::Relaxed);
        let overmastery_offset = OVERMASTERY_OFFSET.load(std::sync::atomic::Ordering::Relaxed);
        let sigil_offset = SIGIL_OFFSET.load(std::sync::atomic::Ordering::Relaxed);

        // If any offset failed to resolve (a game patch broke its signature; setup_globals
        // now logs-and-continues instead of aborting, leaving the offset at its default 0),
        // the struct pointers below would be computed at a1+0 and, worse, the sigil pointer
        // would be read from *(a1+0) — the object's vtable pointer reinterpreted as a
        // SigilList*, which is non-null and so passes the NonNull guard before being
        // dereferenced. Bail rather than read/deref garbage on the game thread.
        if player_offset == 0 || weapon_offset == 0 || overmastery_offset == 0 || sigil_offset == 0
        {
            log::warn!(
                "player_load: skipping, unresolved offset(s) player_data={player_offset:#x} \
                 weapon={weapon_offset:#x} overmastery={overmastery_offset:#x} sigil={sigil_offset:#x}"
            );
            return ret;
        }

        let raw_player_stats = std::ptr::NonNull::new(
            unsafe { a1.byte_add(player_offset as usize) } as *mut PlayerStats,
        );

        let raw_weapon_info = std::ptr::NonNull::new(
            unsafe { a1.byte_add(weapon_offset as usize) } as *mut WeaponInfo,
        );

        let raw_overmastery_info =
            std::ptr::NonNull::new(
                unsafe { a1.byte_add(overmastery_offset as usize) } as *mut Overmasteries
            );

        let sigil_list = std::ptr::NonNull::new(
            unsafe { a1.byte_add(sigil_offset as usize).read() } as *mut SigilList,
        );

        if let (
            Some(raw_player_stats),
            Some(weapon_info),
            Some(overmastery_info),
            Some(sigil_list),
        ) = (
            raw_player_stats,
            raw_weapon_info,
            raw_overmastery_info,
            sigil_list,
        ) {
            let character_type = actor_type_id(a1);
            let player_stats = unsafe { raw_player_stats.as_ref() };
            let weapon_info = unsafe { weapon_info.as_ref() };
            let overmastery_info = unsafe { overmastery_info.as_ref() };
            let sigil_list = unsafe { sigil_list.as_ref() };

            if (sigil_list.party_index as u8) == 0xFF && sigil_list.is_online == 0 {
                return ret;
            }

            let sigils = sigil_list
                .sigils
                .iter()
                .map(|sigil| protocol::Sigil {
                    first_trait_id: sigil.first_trait_id,
                    first_trait_level: sigil.first_trait_level,
                    second_trait_id: sigil.second_trait_id,
                    second_trait_level: sigil.second_trait_level,
                    sigil_id: sigil.sigil_id,
                    equipped_character: sigil.equipped_character,
                    sigil_level: sigil.sigil_level,
                    acquisition_count: sigil.acquisition_count,
                    notification_enum: sigil.notification_enum,
                })
                .collect();

            let character_name = CStr::from_bytes_until_nul(&sigil_list.character_name)
                .ok()
                .map(|cstr| cstr.to_owned())
                .unwrap_or(CString::new("").unwrap());

            let display_name =
                VBuffer(std::ptr::addr_of!(sigil_list.display_name) as *const usize).raw();

            let weapon_info = protocol::WeaponInfo {
                weapon_id: weapon_info.weapon_id,
                star_level: weapon_info.star_level,
                plus_marks: weapon_info.plus_marks,
                awakening_level: weapon_info.awakening_level,
                trait_1_id: weapon_info.trait_1_id,
                trait_1_level: weapon_info.trait_1_level,
                trait_2_id: weapon_info.trait_2_id,
                trait_2_level: weapon_info.trait_2_level,
                trait_3_id: weapon_info.trait_3_id,
                trait_3_level: weapon_info.trait_3_level,
                wrightstone_id: weapon_info.wrightstone_id,
                weapon_level: weapon_info.weapon_level,
                weapon_hp: weapon_info.weapon_hp,
                weapon_attack: weapon_info.weapon_attack,
            };

            let overmastery_info = protocol::OvermasteryInfo {
                overmasteries: overmastery_info
                    .stats
                    .iter()
                    .map(|overmastery| protocol::Overmastery {
                        id: overmastery.id,
                        flags: overmastery.flags,
                        value: overmastery.value,
                    })
                    .collect(),
            };

            let payload = Message::PlayerLoadEvent(protocol::PlayerLoadEvent {
                sigils,
                character_name,
                display_name,
                actor_index: player_idx,
                is_online: sigil_list.is_online != 0,
                party_index: sigil_list.party_index as u8,
                player_stats: protocol::PlayerStats {
                    level: player_stats.level,
                    total_hp: player_stats.total_health,
                    total_attack: player_stats.total_attack,
                    stun_power: player_stats.stun_power,
                    critical_rate: player_stats.critical_rate,
                    total_power: player_stats.total_power,
                },
                character_type,
                weapon_info,
                overmastery_info,
            });

            #[cfg(feature = "console")]
            println!("sending player load event: {:?}", payload);

            let _ = self.tx.send(payload);
        }

        ret
    }
}

// ---------------------------------------------------------------------------
// Game 2.0.2 identity path
//
// The full OnLoadPlayer hook above depends on equipment offsets (sigil/weapon/
// overmastery) that shifted in the 2.0 update and are not yet re-derived, so it
// no longer fires. The identity path below recovers the piece the meter actually
// needs to tell players apart — display name + party slot — from the identity
// snapshot alone, which DID survive the patch.
//
// Two moving parts:
//   1. RefreshPlayerIdentity hook — fires when the game rebuilds a player's
//      identity snapshot. We read the stable name/party fields and cache them
//      keyed by the record's player-key.
//   2. identity_event_for_actor — called from the damage hook with the concrete
//      combat actor. It resolves that actor to a cached identity via the actor's
//      own player-key, and emits a PlayerIdentityEvent.
// ---------------------------------------------------------------------------

/// Offset of the identity snapshot pointer inside the player *record* passed to
/// RefreshPlayerIdentity. VERIFIED on v2.0.2: the hooked function reads
/// `[record + 0x5E60]` as the snapshot base (sigscan + Ghidra decompile of
/// FUN_140a2b600). The snapshot's inner field layout matches [`SigilList`]
/// (is_online/character_name/display_name/party_index at 0x1C8/0x1E8/0x208/0x22C).
const PLAYER_IDENTITY_OFFSET: usize = 0x5E60;

/// Offset of the owning player's key inside the player *record*.
/// UNVERIFIED on this exe — carried over from onelittlechildawa's independent
/// 2.0.2 fix. Read defensively (a wrong value is rejected below, never crashes).
const PLAYER_KEY_OFFSET: usize = 0x5EA8;

/// Offset of the owning player's key inside a concrete combat *actor* (the source
/// instance the damage hook sees). UNVERIFIED on this exe — carried over from the
/// same fork. Read via ReadProcessMemory so a bad range fails the read instead of
/// faulting the game thread.
const ACTOR_PLAYER_KEY_OFFSET: usize = 0x1AB40;

/// Sentinel that the game uses for an unset player key. This is the same
/// `0x887AE0B0` player-data type-hash that anchors the player_data offset scan;
/// it appears where a real key has not been assigned, so treat it as "no key".
const INVALID_PLAYER_KEY: u32 = 0x887A_E0B0;

/// Prologue of the function that rebuilds the player identity snapshot
/// (FUN_140a2b600). VERIFIED unique (1 match) on v2.0.2; clean entry, 1-arg
/// `fn(rcx = player record)`. Hooking the refresh gives us names as soon as a
/// player's identity is (re)built, before the first damage event.
const REFRESH_PLAYER_IDENTITY_SIG: &str =
    "55 41 57 41 56 41 54 56 57 53 48 83 ec 70 48 8d 6c 24 70 48 c7 45 f8 fe ff ff ff 80 b9 bc 5e 00 00 00";

/// Cached identity fields for one player, resolved from a snapshot.
#[derive(Clone, Debug)]
struct StoredPlayerIdentity {
    character_name: CString,
    display_name: CString,
    party_index: u8,
    is_online: bool,
}

/// Player identities keyed by their game player-key, plus a party-slot index so a
/// slot's owner can be replaced (e.g. an offline placeholder giving way to the
/// real remote player as an online lobby fills in).
#[derive(Default)]
struct IdentityStore {
    by_key: HashMap<u32, StoredPlayerIdentity>,
    active_key_by_party: HashMap<u8, u32>,
}

impl IdentityStore {
    /// Records an identity for `player_key`. Returns true if this changed which
    /// key owns the party slot (so cached actor→key mappings must be invalidated).
    fn insert(&mut self, player_key: u32, identity: StoredPlayerIdentity) -> bool {
        let party_index = identity.party_index;
        let previous_key = self.active_key_by_party.insert(party_index, player_key);
        if let Some(previous_key) = previous_key.filter(|key| *key != player_key) {
            self.by_key.remove(&previous_key);
        }
        self.by_key.insert(player_key, identity);

        previous_key != Some(player_key)
    }
}

static IDENTITIES: OnceLock<Mutex<IdentityStore>> = OnceLock::new();
static ACTOR_KEYS: OnceLock<Mutex<HashMap<usize, u32>>> = OnceLock::new();

fn identities() -> &'static Mutex<IdentityStore> {
    IDENTITIES.get_or_init(|| Mutex::new(IdentityStore::default()))
}

fn actor_keys() -> &'static Mutex<HashMap<usize, u32>> {
    ACTOR_KEYS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone)]
pub struct OnLoadPlayerIdentityHook {
    // Retained for symmetry with the other hooks and future use; the identity
    // path publishes through identity_event_for_actor at damage time, not here.
    #[allow(dead_code)]
    tx: event::Tx,
}

impl OnLoadPlayerIdentityHook {
    pub fn new(tx: event::Tx) -> Self {
        Self { tx }
    }

    pub fn setup(&self, process: &Process) -> Result<()> {
        let refresh_player_identity = process
            .search_match_address(REFRESH_PLAYER_IDENTITY_SIG)
            .map_err(|_| anyhow!("Could not find refresh_player_identity"))?;

        #[cfg(feature = "console")]
        println!("Found refresh player identity");

        let cloned_self = self.clone();

        unsafe {
            let func: RefreshPlayerIdentityFunc = std::mem::transmute(refresh_player_identity);
            RefreshPlayerIdentity.initialize(func, move |record| cloned_self.run(record))?;
            RefreshPlayerIdentity.enable()?;
        }

        Ok(())
    }

    fn run(&self, record: *const usize) {
        unsafe { RefreshPlayerIdentity.call(record) };

        if record.is_null() {
            return;
        }

        let snapshot = unsafe {
            (record.byte_add(PLAYER_IDENTITY_OFFSET) as *const *const u8).read_unaligned()
        };
        let player_key = unsafe {
            record
                .byte_add(PLAYER_KEY_OFFSET)
                .cast::<u32>()
                .read_unaligned()
        };

        if player_key == 0 || player_key == INVALID_PLAYER_KEY {
            return;
        }

        let Some(identity) = (unsafe { read_player_identity(snapshot) }) else {
            return;
        };

        // Before an online party is fully populated, the game creates placeholder
        // records for slots 1-3 using the local profile name. They are AI/offline
        // slots, not real remote identities — don't let them shadow real players.
        if !should_cache_identity(&identity) {
            return;
        }

        #[cfg(feature = "console")]
        println!(
            "player identity cached: key={player_key:#010x} party={} online={} name={}",
            identity.party_index,
            identity.is_online,
            identity.display_name.to_string_lossy()
        );

        let mapping_changed = identities()
            .lock()
            .expect("identity map lock poisoned")
            .insert(player_key, identity);

        // Actor allocations get reused as a lobby swaps offline placeholders for
        // the real online party. Force the next hit to re-read the actor's key
        // after any slot mapping change so stale actor→key entries can't persist.
        if mapping_changed {
            actor_keys()
                .lock()
                .expect("actor key map lock poisoned")
                .clear();
        }
    }
}

/// Slot 0 is always the local player and is always kept. Any other slot is only a
/// real player if it is flagged online — offline non-zero slots are placeholders.
fn should_cache_identity(identity: &StoredPlayerIdentity) -> bool {
    identity.party_index == 0 || identity.is_online
}

/// Resolves the concrete combat actor (as seen by the damage hook) to a cached
/// identity, emitting a [`PlayerIdentityEvent`] if one is known.
///
/// Returns `None` when the actor has no resolvable player-key or no identity has
/// been cached for it yet (e.g. an NPC/enemy, or a player whose snapshot has not
/// refreshed). Safe to call for every damage source.
pub fn identity_event_for_actor(
    actor: *const usize,
    character_type: u32,
    actor_index: u32,
) -> Option<PlayerIdentityEvent> {
    if actor.is_null() {
        return None;
    }

    let actor_address = actor as usize;
    let cached_key = actor_keys()
        .lock()
        .expect("actor key map lock poisoned")
        .get(&actor_address)
        .copied();

    let player_key = match cached_key {
        Some(player_key) => player_key,
        None => {
            let player_key = read_actor_player_key(actor)?;
            actor_keys()
                .lock()
                .expect("actor key map lock poisoned")
                .insert(actor_address, player_key);
            player_key
        }
    };

    let identity = identities()
        .lock()
        .expect("identity map lock poisoned")
        .by_key
        .get(&player_key)
        .cloned()?;

    Some(PlayerIdentityEvent {
        character_name: identity.character_name,
        display_name: identity.display_name,
        character_type,
        party_index: identity.party_index,
        actor_index,
        is_online: identity.is_online,
    })
}

/// Reads the player-key from a concrete combat actor via ReadProcessMemory so an
/// invalid/short actor range fails the read rather than faulting the game thread.
fn read_actor_player_key(actor: *const usize) -> Option<u32> {
    let mut player_key = 0u32;
    let mut bytes_read = 0usize;
    let result = unsafe {
        ReadProcessMemory(
            HANDLE(-1),
            actor.byte_add(ACTOR_PLAYER_KEY_OFFSET).cast::<c_void>(),
            (&mut player_key as *mut u32).cast::<c_void>(),
            std::mem::size_of::<u32>(),
            Some(&mut bytes_read),
        )
    };

    if result.is_err()
        || bytes_read != std::mem::size_of::<u32>()
        || player_key == 0
        || player_key == INVALID_PLAYER_KEY
    {
        return None;
    }

    Some(player_key)
}

/// Reads the stable identity fields from a snapshot. Field offsets match the
/// [`SigilList`] layout (verified surviving in v2.0.2): is_online @ 0x1C8,
/// character_name @ 0x1E8, display_name @ 0x208, party_index @ 0x22C.
unsafe fn read_player_identity(snapshot: *const u8) -> Option<StoredPlayerIdentity> {
    if snapshot.is_null() {
        return None;
    }

    let list = &*(snapshot as *const SigilList);

    let is_online = list.is_online;
    let party_index = list.party_index;

    // Reject obviously-bogus snapshots (garbage pointer / wrong struct) so we
    // never cache a junk identity.
    if is_online > 1 || party_index > 3 {
        return None;
    }

    let display_name =
        VBuffer(std::ptr::addr_of!(list.display_name) as *const usize).checked_raw()?;

    // A real player always has a display name; an empty one means this snapshot
    // is not a resolvable identity yet.
    if display_name.as_bytes().is_empty() {
        return None;
    }

    let character_name = CStr::from_bytes_until_nul(&list.character_name)
        .ok()
        .map(|cstr| cstr.to_owned())
        .unwrap_or_else(|| CString::new("").expect("empty CString is valid"));

    Some(StoredPlayerIdentity {
        character_name,
        display_name,
        party_index: party_index as u8,
        is_online: is_online != 0,
    })
}
