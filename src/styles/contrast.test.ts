/// <reference types="node" />
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

// The test runner does not load CSS (css: false), so read the file itself.
// Tests run from the repository root.
const tokensCss = readFileSync(resolve(process.cwd(), "src/styles/tokens.css"), "utf8");

/** Hex color tokens of one theme block, by name without the leading dashes. */
function themeTokens(selector: string): Map<string, string> {
  const start = tokensCss.indexOf(`${selector} {`);
  if (start < 0) throw new Error(`missing theme ${selector}`);
  const end = tokensCss.indexOf("}", start);
  const tokens = new Map<string, string>();
  for (const m of tokensCss.slice(start, end).matchAll(/--([a-z0-9-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)) {
    if (m[1] && m[2]) tokens.set(m[1], m[2]);
  }
  return tokens;
}

/** WCAG 2.2 relative luminance of an sRGB hex color. */
function luminance(hex: string): number {
  const channel = (i: number) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

export function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

const dark = themeTokens('[data-theme="dark"]');
// Paper only overrides colors, so start from the dark values.
const paper = new Map([...dark, ...themeTokens('[data-theme="paper"]')]);

/** Every background that normal-size text is drawn on. */
const BACKGROUNDS = ["bg-app", "bg-sidebar", "bg-surface", "bg-raised", "bg-hover", "bg-selected", "bg-inset"];
/** Tokens used for normal-size text on those backgrounds. */
const TEXT = ["text-primary", "text-secondary", "text-muted", "accent-text", "danger", "success", "warning", "info"];
/** Text on filled controls and tinted badges, including hover and pressed fills. */
const FILLED: Array<[string, string]> = [
  ["text-on-accent", "accent"],
  ["text-on-accent", "accent-hover"],
  ["text-on-accent", "accent-pressed"],
  ["text-on-danger", "danger"],
  ["text-on-danger", "danger-hover"],
  ["text-on-danger", "danger-pressed"],
  ["accent-text", "accent-subtle"],
  ["success", "success-subtle"],
  ["warning", "warning-subtle"],
  ["danger", "danger-subtle"],
  ["info", "info-subtle"],
];

const AA_NORMAL_TEXT = 4.5;

describe("theme contrast", () => {
  it("computes WCAG ratios", () => {
    expect(contrast("#ffffff", "#000000")).toBeCloseTo(21, 5);
    expect(contrast("#777777", "#ffffff")).toBeCloseTo(4.48, 2);
  });

  for (const [name, theme] of [
    ["dark", dark],
    ["paper", paper],
  ] as const) {
    it(`${name}: normal text meets ${AA_NORMAL_TEXT}:1`, () => {
      const pairs: Array<[string, string]> = [];
      for (const fg of TEXT) for (const bg of BACKGROUNDS) pairs.push([fg, bg]);
      pairs.push(...FILLED);
      const failures: string[] = [];
      for (const [fg, bg] of pairs) {
        const a = theme.get(fg);
        const b = theme.get(bg);
        if (!a || !b) throw new Error(`missing token ${a ? bg : fg}`);
        const ratio = contrast(a, b);
        if (ratio < AA_NORMAL_TEXT) failures.push(`${fg} ${a} on ${bg} ${b}: ${ratio.toFixed(2)}`);
      }
      expect(failures).toEqual([]);
    });
  }
});
