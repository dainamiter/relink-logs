//! Offline skill-name backfill logic (pure; driven by `bin/skill_backfill.rs`).
//!
//! `derive_skill_key` reproduces the frontend `getSkillName` character derivation
//! so we can tell, per damage event, which `skills.<char>.<id>` key a name would be
//! looked up under. The ui.json differ then finds ids that resolve nowhere.

use protocol::{ActionType, DamageEvent};

use crate::parser::constants::CharacterType;

/// The lookup coordinates for one skill occurrence: the character block a name
/// would live under (child, then parent as fallback) and the numeric skill id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SkillKey {
    pub child_key: String,
    pub parent_key: String,
    pub id: u32,
}

/// Returns the `SkillKey` for a damage event, or `None` when the event is not a
/// per-character named skill (link/SBA/supplementary) or the character is unknown.
pub fn derive_skill_key(event: &DamageEvent) -> Option<SkillKey> {
    let id = match event.action_id {
        ActionType::Normal(id) | ActionType::DamageOverTime(id) => id,
        _ => return None,
    };

    let parent = CharacterType::from_hash(event.source.parent_actor_type);
    // Seofon's avatar (Pl2200) collapses into Seofon; otherwise the child is the
    // concrete source actor. Mirrors parser/v1/player_state.rs.
    let child = if parent == CharacterType::Pl2200 {
        parent
    } else {
        CharacterType::from_hash(event.source.actor_type)
    };

    let child_key = character_key(child)?;
    let parent_key = character_key(parent)?;
    Some(SkillKey {
        child_key,
        parent_key,
        id,
    })
}

/// A `PlXXXX` key string for a known character, or `None` for `Unknown(_)`
/// (strum renders the inner hash for the default variant, never a `Pl` key).
fn character_key(character: CharacterType) -> Option<String> {
    if matches!(character, CharacterType::Unknown(_)) {
        return None;
    }
    Some(character.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::Actor;

    fn event(parent_hash: u32, actor_hash: u32, action: ActionType) -> DamageEvent {
        DamageEvent {
            source: Actor {
                index: 0,
                actor_type: actor_hash,
                parent_actor_type: parent_hash,
                parent_index: 0,
            },
            target: Actor {
                index: 1,
                actor_type: 0,
                parent_actor_type: 0,
                parent_index: 1,
            },
            action_id: action,
            damage: 100,
            flags: 0,
            attack_rate: None,
            stun_value: None,
            damage_cap: None,
        }
    }

    // Character hashes verified against src-tauri/src/parser/constants.rs from_hash().
    const KATALINA_PL0100: u32 = 0x9498_420D;
    const SEOFON_PL2200: u32 = 0x59DB_0CD9;

    #[test]
    fn normal_skill_yields_child_parent_id() {
        let key = derive_skill_key(&event(
            KATALINA_PL0100,
            KATALINA_PL0100,
            ActionType::Normal(200),
        ))
        .unwrap();
        assert_eq!(key.child_key, "Pl0100");
        assert_eq!(key.parent_key, "Pl0100");
        assert_eq!(key.id, 200);
    }

    #[test]
    fn seofon_avatar_collapses_child_to_parent() {
        // parent = Seofon (Pl2200), child actor = something else -> child collapses to Pl2200.
        let key = derive_skill_key(&event(
            SEOFON_PL2200,
            KATALINA_PL0100,
            ActionType::Normal(1),
        ))
        .unwrap();
        assert_eq!(key.child_key, "Pl2200");
        assert_eq!(key.parent_key, "Pl2200");
    }

    #[test]
    fn link_and_sba_and_supplementary_have_no_key() {
        let k = KATALINA_PL0100;
        assert!(derive_skill_key(&event(k, k, ActionType::LinkAttack)).is_none());
        assert!(derive_skill_key(&event(k, k, ActionType::SBA)).is_none());
        assert!(derive_skill_key(&event(k, k, ActionType::SupplementaryDamage(5))).is_none());
    }

    #[test]
    fn dot_yields_a_key() {
        let k = KATALINA_PL0100;
        let key = derive_skill_key(&event(k, k, ActionType::DamageOverTime(9))).unwrap();
        assert_eq!(key.id, 9);
    }

    #[test]
    fn unknown_character_has_no_key() {
        // 0xDEADBEEF is not a known character hash -> Unknown -> skip.
        assert!(
            derive_skill_key(&event(0xDEAD_BEEF, 0xDEAD_BEEF, ActionType::Normal(1))).is_none()
        );
    }
}
