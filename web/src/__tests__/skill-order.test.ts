import { describe, expect, it } from "vitest";
import type { SkillSummary } from "@/types";
import { orderQuickSkillChips } from "@/lib/skill-order";

function skill(name: string): SkillSummary {
  return { name };
}

describe("quick skill ordering", () => {
  it("prioritizes the required top bar sequence", () => {
    const ordered = orderQuickSkillChips([
      skill("ask-cascade"),
      skill("describe"),
      skill("domain-scaffolder-frontend"),
      skill("deploy"),
      skill("audit-plans"),
      skill("commit"),
      skill("domain-planner"),
      skill("reproduce"),
      skill("domain-reviewer"),
    ]);

    expect(ordered.map((item) => item.name)).toEqual([
      "audit-plans",
      "domain-scaffolder-frontend",
      "domain-planner",
      "domain-reviewer",
      "describe",
      "reproduce",
      "commit",
      "deploy",
      "ask-cascade",
    ]);
  });

  it("keeps non-priority skills in their original relative order", () => {
    const ordered = orderQuickSkillChips([
      skill("zeta"),
      skill("commit"),
      skill("alpha"),
      skill("deploy"),
      skill("audit-plans"),
    ]);

    expect(ordered.map((item) => item.name)).toEqual([
      "audit-plans",
      "commit",
      "deploy",
      "zeta",
      "alpha",
    ]);
  });

  it("handles missing anchor skills without errors", () => {
    const ordered = orderQuickSkillChips([
      skill("domain-planner"),
      skill("ask-cascade"),
      skill("commit"),
    ]);

    expect(ordered.map((item) => item.name)).toEqual([
      "domain-planner",
      "commit",
      "ask-cascade",
    ]);
  });
});
