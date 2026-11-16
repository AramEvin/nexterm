import { describe, expect, it } from "vitest";

import {
  baseName,
  breadcrumbs,
  clickSelection,
  conflictingNames,
  formatBytes,
  formatEta,
  formatMode,
  formatOctal,
  parseOctal,
  sortEntries,
} from "@/lib/files";
import type { FileEntry } from "@/types/files";

const entry = (name: string, kind: FileEntry["kind"], size = 0, modified = 0): FileEntry => ({
  name,
  path: `/d/${name}`,
  kind,
  linkToDir: false,
  size,
  modified,
  permissions: 0o644,
  owner: null,
  group: null,
  hidden: name.startsWith("."),
});

describe("formatting", () => {
  it("formats sizes in binary units", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(5 * 1024 ** 3)).toBe("5.0 GB");
    expect(formatBytes(600 * 1024 ** 2)).toBe("600 MB");
  });

  it("formats ETA", () => {
    expect(formatEta(100, 10)).toBe("10 s");
    expect(formatEta(200, 1)).toBe("3 min 20 s");
    expect(formatEta(7500, 1)).toBe("2 h 05 min");
    expect(formatEta(10, 0)).toBe("");
  });

  it("formats and parses permissions", () => {
    expect(formatMode(0o755)).toBe("rwxr-xr-x");
    expect(formatMode(0o640)).toBe("rw-r-----");
    expect(formatMode(0o4755)).toBe("rwsr-xr-x");
    expect(formatMode(0o1777)).toBe("rwxrwxrwt");
    expect(formatOctal(0o644)).toBe("644");
    expect(formatOctal(0o4755)).toBe("4755");
    expect(parseOctal("0755")).toBe(0o755);
    expect(parseOctal("644")).toBe(0o644);
    expect(parseOctal("899")).toBeNull();
    expect(parseOctal("7")).toBeNull();
  });
});

describe("sorting", () => {
  const items = [
    entry("b.txt", "file", 10, 3),
    entry("Zdir", "dir"),
    entry("a10.txt", "file", 30, 1),
    entry("a2.txt", "file", 20, 2),
    entry("adir", "dir"),
  ];

  it("keeps folders first and sorts names naturally", () => {
    expect(sortEntries(items, { key: "name", asc: true }).map((e) => e.name)).toEqual([
      "adir",
      "Zdir",
      "a2.txt",
      "a10.txt",
      "b.txt",
    ]);
  });

  it("sorts by size descending, folders still first", () => {
    expect(sortEntries(items, { key: "size", asc: false }).map((e) => e.name)).toEqual([
      "Zdir",
      "adir",
      "a10.txt",
      "a2.txt",
      "b.txt",
    ]);
  });
});

describe("selection", () => {
  const paths = ["a", "b", "c", "d", "e"];

  it("click, ctrl-click and shift-click like Explorer", () => {
    let r = clickSelection(paths, new Set(), null, 1, { ctrl: false, shift: false });
    expect([...r.selected]).toEqual(["b"]);
    r = clickSelection(paths, r.selected, r.anchor, 3, { ctrl: false, shift: true });
    expect([...r.selected]).toEqual(["b", "c", "d"]);
    r = clickSelection(paths, r.selected, r.anchor, 2, { ctrl: true, shift: false });
    expect([...r.selected].sort()).toEqual(["b", "d"]);
    r = clickSelection(paths, r.selected, r.anchor, 4, { ctrl: false, shift: false });
    expect([...r.selected]).toEqual(["e"]);
  });
});

describe("paths", () => {
  it("finds name clashes (case-insensitive on Windows disks)", () => {
    const existing = [entry("Report.pdf", "file"), entry("dir", "dir")];
    expect(conflictingNames(["report.pdf", "dir", "new"], existing, false)).toEqual(["dir"]);
    expect(conflictingNames(["report.pdf", "dir", "new"], existing, true)).toEqual([
      "report.pdf",
      "dir",
    ]);
  });

  it("splits paths into breadcrumbs", () => {
    expect(breadcrumbs("/home/u", false)).toEqual([
      { label: "/", path: "/" },
      { label: "home", path: "/home" },
      { label: "u", path: "/home/u" },
    ]);
    expect(breadcrumbs("C:\\Users\\me", true)).toEqual([
      { label: "C:\\", path: "C:\\" },
      { label: "Users", path: "C:\\Users" },
      { label: "me", path: "C:\\Users\\me" },
    ]);
    expect(breadcrumbs("", true)).toEqual([]);
    expect(baseName("C:\\a\\b.txt")).toBe("b.txt");
    expect(baseName("/x/y/")).toBe("y");
  });
});
