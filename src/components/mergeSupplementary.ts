import { SkillState } from "@/types";

type ActionTypeLike = SkillState["actionType"];

export const isSupplementaryAction = (actionType: ActionTypeLike): boolean =>
  typeof actionType === "object" && Object.hasOwn(actionType, "SupplementaryDamage");

export const isDotAction = (actionType: ActionTypeLike): boolean =>
  typeof actionType === "object" && Object.hasOwn(actionType, "DamageOverTime");

/// With the merge toggle on: hide the merged Supplementary Damage row when its
/// damage is fully attributed to trigger skills, otherwise keep only the
/// unattributed remainder so the encounter total still adds up.
export const mergeSupplementaryRows = (skills: SkillState[]): SkillState[] => {
  const attributed = skills.reduce((acc, s) => acc + s.suppDamage + s.echoDamage, 0);

  return skills.flatMap((s) => {
    if (!isSupplementaryAction(s.actionType)) return [s];
    const residual = s.totalDamage - attributed;
    return residual > 0 ? [{ ...s, totalDamage: residual }] : [];
  });
};
