//! Conflux / EndlessMode diagnostic hooks (feature `hookdiag`, off by default).
//!
//! The v2.0.2 "Conflux" roguelike mode is internally codenamed **EndlessMode**
//! (`stage::quest::EndlessModeQuestManager` / `ReceptionEndlessModeFlow`,
//! `ExPlayerEndlessModeBuff`). "Conflux" is a UI/localization string only — it appears
//! nowhere in the exe. See the memory note `gbfr-conflux-endless-mode` for the full map.
//!
//! These two detours exist purely to CAPTURE a live playthrough so the run/room/ability
//! layout can be decoded from the log — there is no protocol/parser wiring yet. Both
//! compile to nothing (empty `setup` returning `Ok(())`) unless `hookdiag` is set. The
//! room-boundary signal comes from the existing `on_load_quest_state` hook (see quest.rs),
//! not from here.
//!
//! Targets (v2.0.2 RVAs, Ghidra-verified clean function entries — see sigs below):
//!   * `FUN_140638690` @ 0x638690 — the quest-reception-flow dispatcher. Given the
//!     stage-quest manager (rcx) and a packed quest-type word (edx), it builds the right
//!     reception flow by `(edx >> 0x14) & 0xf`: **type 8 = ReceptionEndlessModeFlow**
//!     (a Conflux run being set up), type 3 = Fate Episode, etc. 2-arg entry.
//!   * `FUN_14277bc60` @ 0x277bc60 — `ExPlayerEndlessModeBuff::onInstall`, a genuine
//!     per-class virtual method (vtable slot 5). Fires when the endless-mode buff
//!     component is installed on a player actor; it registers ~20 ability slots at
//!     +0xc0, +0x140, +0x1c0, … (stride 0x80). 1-arg entry (rcx = buff `this`).

use anyhow::Result;

use crate::process::Process;

#[cfg(feature = "hookdiag")]
use anyhow::anyhow;
#[cfg(feature = "hookdiag")]
use retour::static_detour;

// v2.0.2 direct-entry signatures (sigscan-verified: 1 match each, cursor at the entry).
// Both anchor on the preceding function's `ret` + int3 padding, then the target prologue.
//   dispatcher: c3 cc cc | 55 56 57 53 48 83 ec 38 ...  (0x638690)
//   onInstall:  c3 cc cc cc | 56 57 48 83 ec 68 48 89 ce ...  (0x277bc60)
#[cfg(feature = "hookdiag")]
const ON_RECEPTION_FLOW_DISPATCH_SIG: &str =
    "c3 cc cc ' 55 56 57 53 48 83 ec 38 48 8d 6c 24 30 48 c7 45 00 fe ff ff ff 48 89 ce c1 ea 14";
#[cfg(feature = "hookdiag")]
const ON_ENDLESS_BUFF_INSTALL_SIG: &str =
    "c3 cc cc cc ' 56 57 48 83 ec 68 48 89 ce 48 8d 91 c0 00 00 00 48 8d 05";
// EndlessModeQuestManager destructor FUN_14060d7b0 (0x60d7b0), a clean 1-arg entry
// `fn(rcx=manager)`. The manager is created at run start and freed ONCE at run end (its
// base stayed stable across all rooms in a run — only the per-room reception FLOW churns),
// so this is the unambiguous run-END signal that no reception fires for (the reward
// screen / exit-to-town path doesn't go through the reception dispatcher). Sig anchors on
// the preceding `ret`+int3 padding then the distinctive large-frame prologue; sigscan = 1
// match resolving to 0x60d7b0.
#[cfg(feature = "hookdiag")]
const ON_ENDLESS_MGR_DTOR_SIG: &str =
    "cc cc cc cc ' 55 41 57 41 56 41 55 41 54 56 57 53 48 81 ec 78 04 00 00 48 8d ac 24 80 00 00 00 c5 f8 29 bd e0 03 00 00";

/// Quest-type value (decoded `(edx >> 0x14) & 0xf` in the dispatcher) that selects the
/// EndlessMode reception flow — i.e. a Conflux run being set up.
#[cfg(feature = "hookdiag")]
const QUEST_TYPE_ENDLESS_MODE: u32 = 8;

#[cfg(feature = "hookdiag")]
type OnReceptionFlowDispatchFunc = unsafe extern "system" fn(*const usize, u32) -> usize;
#[cfg(feature = "hookdiag")]
type OnEndlessBuffInstallFunc = unsafe extern "system" fn(*const usize) -> usize;
#[cfg(feature = "hookdiag")]
type OnEndlessMgrDtorFunc = unsafe extern "system" fn(*const usize) -> usize;

#[cfg(feature = "hookdiag")]
static_detour! {
    static OnReceptionFlowDispatch: unsafe extern "system" fn(*const usize, u32) -> usize;
    static OnEndlessBuffInstall: unsafe extern "system" fn(*const usize) -> usize;
    static OnEndlessMgrDtor: unsafe extern "system" fn(*const usize) -> usize;
}

/// Logs each quest-reception-flow build, flagging quest-type 8 = EndlessMode (Conflux run
/// setup). Observe-only: both args are passed straight through (a dropped arg crashed the
/// quest hook on v2.0.2 — see quest.rs).
#[derive(Clone)]
pub struct OnReceptionFlowDispatchHook {}

impl OnReceptionFlowDispatchHook {
    pub fn new() -> Self {
        OnReceptionFlowDispatchHook {}
    }

    #[cfg(feature = "hookdiag")]
    pub fn setup(&self, process: &Process) -> Result<()> {
        let cloned_self = self.clone();

        if let Ok(addr) = process.search_address(ON_RECEPTION_FLOW_DISPATCH_SIG) {
            unsafe {
                let func: OnReceptionFlowDispatchFunc = std::mem::transmute(addr);
                OnReceptionFlowDispatch
                    .initialize(func, move |a1, a2| cloned_self.run(a1, a2))?;
                OnReceptionFlowDispatch.enable()?;
            }
            Ok(())
        } else {
            Err(anyhow!("Could not find reception_flow_dispatch"))
        }
    }

    #[cfg(not(feature = "hookdiag"))]
    pub fn setup(&self, _process: &Process) -> Result<()> {
        Ok(())
    }

    #[cfg(feature = "hookdiag")]
    fn run(&self, a1: *const usize, a2: u32) -> usize {
        // a2 is a packed quest-type word; the dispatcher selects the flow by (a2>>0x14)&0xf.
        let quest_type = (a2 >> 0x14) & 0xf;

        // Read the reception-flow singleton slot (+0x210) BEFORE calling the original — the
        // dispatcher REPLACES it, so the pre-call pointer is the OUTGOING flow. Comparing it
        // to the post-call pointer is how run boundaries surface: a run STARTS when the slot
        // goes null -> EndlessMode-flow, and ENDS when it goes EndlessMode-flow -> other/null
        // (e.g. entering/leaving the Conflux hub, which dispatches a NON-8 quest_type). If we
        // only probed on quest_type==8 we'd never see those boundary transitions, so probe on
        // EVERY reception now.
        let flow_before = unsafe { a1.byte_add(0x210).read() };
        // The flow stamps its type-hash at flow+0x7c8 (puVar5[0xf9] in FUN_140638690:
        // 0xf9*8 = 0x7c8). Log it so we can tell an EndlessMode flow (0x887ae0b0) apart from
        // a Fate/other flow across the boundary. Guarded read (0 if unreadable).
        let flow_type_before = crate::hooks::diag::read_u32_guarded(flow_before, 0x7c8);
        crate::hooks::diag::ev!(
            "endless_reception",
            "raw={a2:#x} quest_type={quest_type} is_endless={} flow_before={flow_before:#x} flow_type_before={flow_type_before:#x}",
            quest_type == QUEST_TYPE_ENDLESS_MODE
        );

        // Delta-probe the persistent run-state manager on EVERY reception (not just type 8) so
        // fields that flip at run start/end — e.g. manager+0x208 (run-progress flag) and the
        // reception-flow slot at +0x210 — are captured even when the boundary transition uses
        // a non-EndlessMode quest_type.
        crate::hooks::diag::probe_u32_window_delta("reception_mgr", a1 as usize, 0x800);
        if flow_before != 0 {
            crate::hooks::diag::probe_u32_window_delta("reception_flow", flow_before, 0x820);
        }

        let ret = unsafe { OnReceptionFlowDispatch.call(a1, a2) };

        // After the dispatcher runs, log the NEW flow + its type so a null->flow (run start)
        // or flow->null/other (run end) transition is unambiguous in the log.
        let flow_after = unsafe { a1.byte_add(0x210).read() };
        let flow_type_after = crate::hooks::diag::read_u32_guarded(flow_after, 0x7c8);
        crate::hooks::diag::ev!(
            "endless_reception_after",
            "quest_type={quest_type} flow_after={flow_after:#x} flow_type_after={flow_type_after:#x} changed={}",
            flow_after != flow_before
        );

        ret
    }
}

/// Logs each `ExPlayerEndlessModeBuff::onInstall` and dumps the buff component's field
/// window, so a playthrough reveals which ability slots (+0xc0, +0x140, … stride 0x80)
/// populate as abilities are picked. Observe-only pass-through.
#[derive(Clone)]
pub struct OnEndlessBuffInstallHook {}

impl OnEndlessBuffInstallHook {
    pub fn new() -> Self {
        OnEndlessBuffInstallHook {}
    }

    #[cfg(feature = "hookdiag")]
    pub fn setup(&self, process: &Process) -> Result<()> {
        let cloned_self = self.clone();

        if let Ok(addr) = process.search_address(ON_ENDLESS_BUFF_INSTALL_SIG) {
            unsafe {
                let func: OnEndlessBuffInstallFunc = std::mem::transmute(addr);
                OnEndlessBuffInstall.initialize(func, move |a1| cloned_self.run(a1))?;
                OnEndlessBuffInstall.enable()?;
            }
            Ok(())
        } else {
            Err(anyhow!("Could not find endless_buff_install"))
        }
    }

    #[cfg(not(feature = "hookdiag"))]
    pub fn setup(&self, _process: &Process) -> Result<()> {
        Ok(())
    }

    #[cfg(feature = "hookdiag")]
    fn run(&self, a1: *const usize) -> usize {
        crate::hooks::diag::ev!("endless_buff", "buff_this={:#x}", a1 as usize);
        // onInstall is the buff-RNG init (MT19937: seed at +0xbc0, 624-word state at +0xbc4),
        // so on the first hit the ability slots are still empty. The same buff object is
        // re-visited (onInstall fires again), so use the delta probe to catch fields that get
        // populated later. Window widened to 0x1000 to include the RNG seed region.
        crate::hooks::diag::probe_u32_window_delta("endless_buff", a1 as usize, 0x1000);
        unsafe { OnEndlessBuffInstall.call(a1) }
    }
}

/// Logs when the `EndlessModeQuestManager` is destroyed — the unambiguous **run-END**
/// signal. The reward-screen / exit-to-town path does NOT fire a reception, so run-end has
/// no reception event; the manager (persistent for the whole run, unlike the per-room flow)
/// is freed exactly once when the run concludes, making its destructor the clean marker.
/// Observe-only: `a1` (the manager) is logged and a final delta-probe captures its end-state,
/// then the real destructor runs unchanged.
#[derive(Clone)]
pub struct OnEndlessMgrDtorHook {}

impl OnEndlessMgrDtorHook {
    pub fn new() -> Self {
        OnEndlessMgrDtorHook {}
    }

    #[cfg(feature = "hookdiag")]
    pub fn setup(&self, process: &Process) -> Result<()> {
        let cloned_self = self.clone();

        if let Ok(addr) = process.search_address(ON_ENDLESS_MGR_DTOR_SIG) {
            unsafe {
                let func: OnEndlessMgrDtorFunc = std::mem::transmute(addr);
                OnEndlessMgrDtor.initialize(func, move |a1| cloned_self.run(a1))?;
                OnEndlessMgrDtor.enable()?;
            }
            Ok(())
        } else {
            Err(anyhow!("Could not find endless_mgr_dtor"))
        }
    }

    #[cfg(not(feature = "hookdiag"))]
    pub fn setup(&self, _process: &Process) -> Result<()> {
        Ok(())
    }

    #[cfg(feature = "hookdiag")]
    fn run(&self, a1: *const usize) -> usize {
        crate::hooks::diag::ev!("endless_run_end", "manager={:#x}", a1 as usize);
        // Final state of the run object right before teardown (delta vs the last reception
        // snapshot for the same base — shows what changed over the run's final leg). Probe
        // BEFORE calling the destructor, while the memory is still valid.
        crate::hooks::diag::probe_u32_window_delta("reception_mgr", a1 as usize, 0x800);
        unsafe { OnEndlessMgrDtor.call(a1) }
    }
}
