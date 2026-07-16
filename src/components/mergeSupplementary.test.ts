import { describe, expect, it } from "vitest";
import { SkillState } from "@/types";
import { isDotAction, isSupplementaryAction, mergeSupplementaryRows } from "./mergeSupplementary";

const skill = (overrides: Partial<SkillState>): SkillState => ({
  actionType: { Normal: 1 },
  childCharacterType: "Pl0000",
  hits: 1,
  minDamage: 100,
  maxDamage: 100,
  totalDamage: 100,
  totalStunValue: 0,
  maxStunValue: 0,
  cappedHits: 0,
  cappableHits: 0,
  suppHits: 0,
  echoHits: 0,
  suppDamage: 0,
  echoDamage: 0,
  ...overrides,
});

describe("isSupplementaryAction / isDotAction", () => {
  it("detects the action families", () => {
    expect(isSupplementaryAction({ SupplementaryDamage: 5 })).toBe(true);
    expect(isSupplementaryAction({ Normal: 5 })).toBe(false);
    expect(isSupplementaryAction("LinkAttack")).toBe(false);
    expect(isDotAction({ DamageOverTime: 0 })).toBe(true);
    expect(isDotAction({ Normal: 5 })).toBe(false);
  });
});

describe("mergeSupplementaryRows", () => {
  it("drops the merged supp row when its damage is fully attributed", () => {
    const rows = [
      skill({ actionType: { Normal: 1 }, totalDamage: 1000, suppDamage: 150, echoDamage: 50 }),
      skill({ actionType: { SupplementaryDamage: 1 }, totalDamage: 200 }),
    ];
    const merged = mergeSupplementaryRows(rows);
    expect(merged).toHaveLength(1);
    expect(merged[0].actionType).toEqual({ Normal: 1 });
  });

  it("keeps a residual row when some proc damage is unattributed", () => {
    const rows = [
      skill({ actionType: { Normal: 1 }, totalDamage: 1000, suppDamage: 150 }),
      skill({ actionType: { SupplementaryDamage: 1 }, totalDamage: 200 }),
    ];
    const merged = mergeSupplementaryRows(rows);
    expect(merged).toHaveLength(2);
    const residual = merged.find((s) => isSupplementaryAction(s.actionType));
    expect(residual?.totalDamage).toBe(50);
  });

  it("leaves rows unchanged when there is no supp row", () => {
    const rows = [skill({ actionType: { Normal: 1 }, totalDamage: 1000 })];
    expect(mergeSupplementaryRows(rows)).toEqual(rows);
  });
});
