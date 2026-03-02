import type { SkillSummary } from "@/types";

function quickSkillPriority(name: string): number {
  const normalized = name.trim().toLowerCase();
  if (normalized === "audit-plans") return 0;
  if (normalized.startsWith("domain-")) return 1;
  if (normalized === "describe") return 2;
  if (normalized === "reproduce") return 3;
  if (normalized === "commit") return 4;
  if (normalized === "deploy-debug") return 5;
  return 6;
}

export function orderQuickSkillChips(skills: SkillSummary[]): SkillSummary[] {
  return skills
    .map((skill, index) => ({
      skill,
      index,
      priority: quickSkillPriority(skill.name),
    }))
    .sort((a, b) => {
      if (a.priority !== b.priority) return a.priority - b.priority;
      return a.index - b.index;
    })
    .map((entry) => entry.skill);
}
