import { beforeAll, afterAll, describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const testDir = dirname(fileURLToPath(import.meta.url));
const css = readFileSync(resolve(testDir, "../style.css"), "utf8");

function px(value: string): number {
  const parsed = Number.parseFloat(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

describe("mobile touch target CSS guards", () => {
  let styleEl: HTMLStyleElement;

  beforeAll(() => {
    styleEl = document.createElement("style");
    styleEl.textContent = css;
    document.head.appendChild(styleEl);
  });

  afterAll(() => {
    styleEl.remove();
  });

  it("close hit target stays at least 44x44", () => {
    const closeBtn = document.createElement("button");
    closeBtn.className = "zone-close-hitbox";
    document.body.appendChild(closeBtn);

    const styles = getComputedStyle(closeBtn);
    expect(px(styles.width)).toBeGreaterThanOrEqual(44);
    expect(px(styles.height)).toBeGreaterThanOrEqual(44);
    expect(px(styles.minWidth || styles.width)).toBeGreaterThanOrEqual(44);
    expect(px(styles.minHeight || styles.height)).toBeGreaterThanOrEqual(44);

    closeBtn.remove();
  });

  it("mobile keybar buttons keep at least 44x44 touch targets", () => {
    const keyBtn = document.createElement("button");
    keyBtn.className = "mobile-keybar-btn";
    keyBtn.textContent = "Tab";
    document.body.appendChild(keyBtn);

    const styles = getComputedStyle(keyBtn);
    expect(px(styles.minWidth)).toBeGreaterThanOrEqual(44);
    expect(px(styles.minHeight)).toBeGreaterThanOrEqual(44);

    keyBtn.remove();
  });
});
