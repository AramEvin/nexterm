import { describe, expect, it } from "vitest";

import {
  buildIndex,
  countSessions,
  flattenTree,
  folderAndDescendants,
  folderPath,
  parseTags,
  searchSessions,
} from "@/lib/library";
import type { Folder, SavedSession } from "@/types/library";

let n = 0;
const folder = (name: string, parentId: string | null = null, sortOrder = 0): Folder => ({
  id: `f-${name}`,
  parentId,
  name,
  defaultCredentialId: null,
  sortOrder,
  createdAt: 0,
  updatedAt: 0,
});
const session = (
  name: string,
  folderId: string | null,
  extra: Partial<SavedSession> = {},
): SavedSession => ({
  id: `s-${name}-${n++}`,
  folderId,
  name,
  protocol: "ssh",
  host: `${name}.corp.local`,
  port: 22,
  username: "root",
  authMethod: "password",
  credentialId: null,
  jumpHostId: null,
  color: null,
  notes: "",
  favorite: false,
  tags: [],
  sortOrder: 0,
  createdAt: 0,
  updatedAt: 0,
  lastConnectedAt: null,
  ...extra,
});

const prod = folder("Prod", null, 0);
const web = folder("Web", prod.id, 0);
const lab = folder("Lab", null, 1);
const folders = [lab, web, prod];
const sessions = [
  session("web-01", web.id, { tags: ["nginx", "eu"] }),
  session("web-02", web.id, { tags: ["nginx"] }),
  session("db-01", prod.id, { tags: ["postgres", "eu"], username: "dba" }),
  session("pi", null, { host: "192.168.1.50" }),
];
const index = buildIndex(folders, sessions);

describe("tree", () => {
  it("orders folders by sortOrder and nests children", () => {
    const rows = flattenTree(index, new Set([prod.id, web.id]));
    expect(rows.map((r) => (r.kind === "folder" ? `F:${r.folder.name}` : r.session.name))).toEqual([
      "F:Prod",
      "F:Web",
      "web-01",
      "web-02",
      "db-01",
      "F:Lab",
      "pi",
    ]);
    expect(rows.map((r) => r.depth)).toEqual([0, 1, 2, 2, 1, 0, 0]);
  });

  it("collapsed folders hide their contents", () => {
    const rows = flattenTree(index, new Set());
    expect(rows.map((r) => (r.kind === "folder" ? r.folder.name : r.session.name))).toEqual([
      "Prod",
      "Lab",
      "pi",
    ]);
  });

  it("counts sessions recursively", () => {
    expect(countSessions(index, prod.id)).toBe(3);
    expect(countSessions(index, lab.id)).toBe(0);
  });

  it("builds folder paths and descendant sets", () => {
    expect(folderPath(index, web.id)).toBe("Prod / Web");
    expect(folderPath(index, null)).toBe("");
    expect([...folderAndDescendants(index, prod.id)].sort()).toEqual([prod.id, web.id].sort());
  });

  it("orphaned sessions show at the top level instead of disappearing", () => {
    const orphan = session("lost", "f-missing");
    const rows = flattenTree(buildIndex(folders, [orphan]), new Set());
    expect(
      rows.some((r) => r.kind === "session" && r.session.name === "lost" && r.depth === 0),
    ).toBe(true);
  });
});

describe("search", () => {
  const names = (q: string) => searchSessions(index, sessions, q).map((s) => s.name);

  it("groups results by folder path (root first)", () => {
    // db-01 lives in "Prod", web-01 in "Prod / Web".
    expect(names("01")).toEqual(["db-01", "web-01"]);
  });

  it("matches name, host, user, folder path and tags", () => {
    expect(names("web")).toEqual(["web-01", "web-02"]); // name/host/folder
    expect(names("192.168")).toEqual(["pi"]); // host
    expect(names("dba")).toEqual(["db-01"]); // username
    expect(names("postgres")).toEqual(["db-01"]); // tag
    expect(names("prod")).toEqual(["db-01", "web-01", "web-02"]); // folder path, grouped
  });

  it("all terms must match; tag: restricts to tags; case-insensitive", () => {
    expect(names("EU nginx")).toEqual(["web-01"]);
    expect(names("tag:eu")).toEqual(["db-01", "web-01"]);
    expect(names("tag:root")).toEqual([]); // 'root' is a username, not a tag
    expect(names("   ")).toEqual([]);
  });

  it("searches 1000 sessions in well under a frame", () => {
    const many = Array.from({ length: 1000 }, (_, i) =>
      session(`srv-${i}`, i % 2 ? prod.id : web.id, { tags: [`rack${i % 20}`] }),
    );
    const bigIndex = buildIndex(folders, many);
    const start = performance.now();
    const hits = searchSessions(bigIndex, many, "srv-99 rack19");
    const elapsed = performance.now() - start;
    expect(hits.map((s) => s.name)).toEqual(["srv-99", "srv-999"]);
    expect(elapsed).toBeLessThan(50);
  });
});

describe("parseTags", () => {
  it("splits on commas, trims, drops empties and duplicates", () => {
    expect(parseTags(" prod, db ,, Prod , eu-west ")).toEqual(["prod", "db", "eu-west"]);
    expect(parseTags("")).toEqual([]);
  });
});
