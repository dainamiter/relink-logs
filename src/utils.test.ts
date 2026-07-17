import { describe, expect, it } from "vitest";
import { PlayerState, SkillState } from "./types";
import { computeOvercapPercentage, computeSupPercentage, toHash, toHashString } from "./utils";

const makeSkill = (actionType: SkillState["actionType"], totalDamage: number): SkillState => ({
  actionType,
  childCharacterType: "Pl0000",
  hits: 1,
  minDamage: totalDamage,
  maxDamage: totalDamage,
  totalDamage,
  totalStunValue: 0,
  maxStunValue: 0,
  cappedHits: 0,
  cappableHits: 0,
  overcapBaseSum: 0,
  overcapCapSum: 0,
});

const makePlayer = (skills: SkillState[]): PlayerState => ({
  index: 0,
  characterType: "Pl0000",
  totalDamage: skills.reduce((acc, s) => acc + s.totalDamage, 0),
  dps: 0,
  sba: 0,
  totalStunValue: 0,
  stunPerSecond: 0,
  lastDamageTime: 0,
  skillBreakdown: skills,
  cappedHits: 0,
  cappableHits: 0,
  overcapBaseSum: 0,
  overcapCapSum: 0,
});

describe("utils", () => {
  it("toHash", () => {
    expect(toHash(1)).toBe("1");
    expect(toHash(255)).toBe("ff");
  });

  it("toHashString", () => {
    expect(toHashString(1)).toBe("00000001");
    expect(toHashString(255)).toBe("000000ff");
  });

  describe("computeSupPercentage", () => {
    it("is 0 without supplementary damage", () => {
      const player = makePlayer([makeSkill({ Normal: 1 }, 1000)]);
      expect(computeSupPercentage(player)).toEqual({ eligible: 0, overall: 0 });
    });

    it("is supplementary damage relative to eligible damage", () => {
      // 1000 eligible + 200 procs -> +20% eligible; 200 of 1200 total -> ~16.7% overall
      const player = makePlayer([makeSkill({ Normal: 1 }, 1000), makeSkill({ SupplementaryDamage: 1 }, 200)]);
      const { eligible, overall } = computeSupPercentage(player);
      expect(eligible).toBeCloseTo(20);
      expect(overall).toBeCloseTo(100 / 6);
    });

    it("excludes supp-ineligible damage (Link Attack, SBA, DoT) from the eligible base", () => {
      // 1000 eligible + 200 procs -> +20% eligible, regardless of LA/SBA/DoT damage;
      // overall is the supp share of ALL damage
      const player = makePlayer([
        makeSkill({ Normal: 1 }, 1000),
        makeSkill({ SupplementaryDamage: 1 }, 200),
        makeSkill("LinkAttack", 800),
        makeSkill("SBA", 6000),
        makeSkill({ DamageOverTime: 0 }, 2000),
      ]);
      const { eligible, overall } = computeSupPercentage(player);
      expect(eligible).toBeCloseTo(20);
      expect(overall).toBeCloseTo((200 / 10000) * 100);
    });

    it("caps out at +60% when every hit procs all three sources", () => {
      const player = makePlayer([makeSkill({ Normal: 1 }, 1000), makeSkill({ SupplementaryDamage: 1 }, 600)]);
      expect(computeSupPercentage(player).eligible).toBeCloseTo(60);
    });

    it("is 0 for a player with no damage", () => {
      const player = makePlayer([]);
      expect(computeSupPercentage(player)).toEqual({ eligible: 0, overall: 0 });
    });
  });

  describe("computeOvercapPercentage", () => {
    it("is the game's (ΣbaseSum / ΣcapSum) * 100", () => {
      // base 1500 vs cap 1000 -> 150%
      expect(computeOvercapPercentage({ overcapBaseSum: 1500, overcapCapSum: 1000 })).toBeCloseTo(150);
      // exactly at cap -> 100%
      expect(computeOvercapPercentage({ overcapBaseSum: 1000, overcapCapSum: 1000 })).toBeCloseTo(100);
    });

    it("is null when there are no cappable hits (no cap sum)", () => {
      expect(computeOvercapPercentage({ overcapBaseSum: 0, overcapCapSum: 0 })).toBeNull();
    });
  });
});
