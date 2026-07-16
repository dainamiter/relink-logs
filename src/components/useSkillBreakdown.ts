import { useShallow } from "zustand/react/shallow";

import SkillGroupMapping from "@/assets/skill-groups";
import { useMeterSettingsStore } from "@/stores/useMeterSettingsStore";
import { ComputedPlayerState, ComputedSkillGroup, ComputedSkillState } from "@/types";
import { getSkillName } from "@/utils";
import { mergeSupplementaryRows } from "./mergeSupplementary";

export const useSkillBreakdown = (player: ComputedPlayerState) => {
  const { useCondensedSkills, mergeSupplementary } = useMeterSettingsStore(
    useShallow((state) => ({
      useCondensedSkills: state.use_condensed_skills,
      mergeSupplementary: state.merge_supplementary,
    }))
  );

  // Denominator: the player's full damage including the merged supp row, so
  // percentages are identical whether or not the row is folded in.
  const totalDamage = player.skillBreakdown.reduce((acc, skill) => acc + skill.totalDamage, 0);
  const breakdown = mergeSupplementary ? mergeSupplementaryRows(player.skillBreakdown) : player.skillBreakdown;
  const computedSkills = breakdown.map<ComputedSkillState>((skill) => {
    const totalDisplayDamage = skill.totalDamage + (mergeSupplementary ? skill.suppDamage + skill.echoDamage : 0);
    return {
      percentage: (totalDisplayDamage / totalDamage) * 100,
      totalDisplayDamage,
      suppPercentage: mergeSupplementary ? (skill.suppDamage / totalDamage) * 100 : 0,
      echoPercentage: mergeSupplementary ? (skill.echoDamage / totalDamage) * 100 : 0,
      groupName: getSkillName(player.characterType, skill),
      ...skill,
    };
  });

  let skillsToShow: Array<ComputedSkillGroup | ComputedSkillState> = computedSkills;

  if (useCondensedSkills && typeof player.characterType == "string") {
    const skills: Array<ComputedSkillGroup | ComputedSkillGroup> = [];

    for (const skill of computedSkills) {
      const skillGroupIndex = typeof skill.childCharacterType !== "string" ? -1 : skill.childCharacterType;
      const skillGroupMapping = SkillGroupMapping[skillGroupIndex] || {};

      if (typeof skill.actionType == "object" && Object.hasOwn(skill.actionType, "Normal")) {
        const actionType = skill.actionType as { Normal: number };
        let wasGroupedSkill = false;

        for (const group in skillGroupMapping) {
          const groupActionType = { Group: group };
          const skillBelongsToGroup = skillGroupMapping[group].skills.includes(actionType.Normal);

          if (skillBelongsToGroup) {
            const skillGroupIndex = skills.findIndex((skillGroup) => {
              if (typeof skillGroup.actionType === "object" && Object.hasOwn(skillGroup.actionType, "Group")) {
                const actionType = skillGroup.actionType as { Group: string };

                return actionType.Group == group && skillGroup.childCharacterType == skill.childCharacterType;
              } else {
                return false;
              }
            });

            if (skillGroupIndex >= 0) {
              const skillGroup = skills[skillGroupIndex] as ComputedSkillGroup;

              skills[skillGroupIndex] = {
                ...skillGroup,
                hits: skillGroup.hits + skill.hits,
                cappedHits: skillGroup.cappedHits + skill.cappedHits,
                cappableHits: skillGroup.cappableHits + skill.cappableHits,
                percentage: skillGroup.percentage + skill.percentage,
                totalDamage: skillGroup.totalDamage + skill.totalDamage,
                minDamage: Math.min(skillGroup?.minDamage || 0, skill.minDamage || 0),
                maxDamage: Math.max(skillGroup?.maxDamage ?? Number.MIN_VALUE, skill.maxDamage || 0),
                suppHits: skillGroup.suppHits + skill.suppHits,
                echoHits: skillGroup.echoHits + skill.echoHits,
                suppDamage: skillGroup.suppDamage + skill.suppDamage,
                echoDamage: skillGroup.echoDamage + skill.echoDamage,
                totalDisplayDamage: skillGroup.totalDisplayDamage + skill.totalDisplayDamage,
                suppPercentage: skillGroup.suppPercentage + skill.suppPercentage,
                echoPercentage: skillGroup.echoPercentage + skill.echoPercentage,
                skills: [...(skillGroup.skills || []), skill],
              };
            } else {
              skills.push({
                actionType: groupActionType,
                childCharacterType: skill.childCharacterType,
                hits: skill.hits,
                cappedHits: skill.cappedHits,
                cappableHits: skill.cappableHits,
                totalDamage: skill.totalDamage,
                minDamage: skill.minDamage,
                maxDamage: skill.maxDamage,
                percentage: skill.percentage,
                skills: [skill],
                maxStunValue: skill.maxStunValue,
                totalStunValue: skill.totalStunValue,
                suppHits: skill.suppHits,
                echoHits: skill.echoHits,
                suppDamage: skill.suppDamage,
                echoDamage: skill.echoDamage,
                totalDisplayDamage: skill.totalDisplayDamage,
                suppPercentage: skill.suppPercentage,
                echoPercentage: skill.echoPercentage,
              });
            }

            wasGroupedSkill = true;

            break;
          }
        }

        if (!wasGroupedSkill) {
          skills.push(skill);
        }
      } else {
        skills.push(skill);
      }
    }

    skillsToShow = skills;
  }

  skillsToShow.sort((a, b) => b.totalDisplayDamage - a.totalDisplayDamage);

  return {
    skills: skillsToShow,
  };
};
