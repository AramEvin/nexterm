/**
 * Pure helpers for the session library: tree building, search, paths.
 * No React, no IPC, so they are cheap to unit-test.
 */
import type { Folder, SavedSession } from "@/types/library";

export type TreeRow =
  | { kind: "folder"; folder: Folder; depth: number; expanded: boolean; count: number }
  | { kind: "session"; session: SavedSession; depth: number };

/** Children lookup: parent id (or "" for root) -> folders / sessions, in sort order. */
export interface LibraryIndex {
  foldersByParent: Map<string, Folder[]>;
  sessionsByFolder: Map<string, SavedSession[]>;
  folderById: Map<string, Folder>;
}

const ROOT = "";
const byOrder = <T extends { sortOrder: number; name: string }>(a: T, b: T) =>
  a.sortOrder - b.sortOrder || a.name.localeCompare(b.name, undefined, { sensitivity: "base" });

export function buildIndex(folders: Folder[], sessions: SavedSession[]): LibraryIndex {
  const foldersByParent = new Map<string, Folder[]>();
  const sessionsByFolder = new Map<string, SavedSession[]>();
  const folderById = new Map<string, Folder>();
  for (const f of folders) {
    folderById.set(f.id, f);
    const key = f.parentId ?? ROOT;
    const list = foldersByParent.get(key) ?? [];
    list.push(f);
    foldersByParent.set(key, list);
  }
  for (const s of sessions) {
    // A session whose folder vanished is shown at the root rather than lost.
    const key = s.folderId && folderById.has(s.folderId) ? s.folderId : ROOT;
    const list = sessionsByFolder.get(key) ?? [];
    list.push(s);
    sessionsByFolder.set(key, list);
  }
  for (const list of foldersByParent.values()) list.sort(byOrder);
  for (const list of sessionsByFolder.values()) list.sort(byOrder);
  return { foldersByParent, sessionsByFolder, folderById };
}

/** Number of sessions in a folder, including all subfolders. */
export function countSessions(index: LibraryIndex, folderId: string): number {
  let total = index.sessionsByFolder.get(folderId)?.length ?? 0;
  for (const child of index.foldersByParent.get(folderId) ?? []) {
    total += countSessions(index, child.id);
  }
  return total;
}

/** Visible rows of the tree (folders first, then sessions, depth-first). */
export function flattenTree(index: LibraryIndex, expanded: ReadonlySet<string>): TreeRow[] {
  const rows: TreeRow[] = [];
  const walk = (parent: string, depth: number) => {
    for (const folder of index.foldersByParent.get(parent) ?? []) {
      const isOpen = expanded.has(folder.id);
      rows.push({
        kind: "folder",
        folder,
        depth,
        expanded: isOpen,
        count: countSessions(index, folder.id),
      });
      if (isOpen) walk(folder.id, depth + 1);
    }
    for (const session of index.sessionsByFolder.get(parent) ?? []) {
      rows.push({ kind: "session", session, depth });
    }
  };
  walk(ROOT, 0);
  return rows;
}

/** "Prod / Web" for a folder id; "" for the root. */
export function folderPath(index: LibraryIndex, folderId: string | null): string {
  const parts: string[] = [];
  let current = folderId ? index.folderById.get(folderId) : undefined;
  let guard = 0;
  while (current && guard++ < 64) {
    parts.unshift(current.name);
    current = current.parentId ? index.folderById.get(current.parentId) : undefined;
  }
  return parts.join(" / ");
}

/** Ids of a folder and all its descendants (for "can't drop into itself"). */
export function folderAndDescendants(index: LibraryIndex, folderId: string): Set<string> {
  const out = new Set<string>([folderId]);
  const walk = (id: string) => {
    for (const child of index.foldersByParent.get(id) ?? []) {
      out.add(child.id);
      walk(child.id);
    }
  };
  walk(folderId);
  return out;
}

/** Lower-cased text a session is searched by. */
export function searchText(session: SavedSession, path: string): string {
  return [session.name, session.host, session.username, path, ...session.tags]
    .join("\n")
    .toLowerCase();
}

/**
 * Every whitespace-separated term must match somewhere (name, host, user,
 * folder path or a tag). `tag:x` matches tags only. Results are grouped by
 * folder path, then by their order inside the folder.
 */
export function searchSessions(
  index: LibraryIndex,
  sessions: SavedSession[],
  query: string,
): SavedSession[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return [];
  const paths = new Map<string, string>();
  const pathOf = (folderId: string | null) => {
    const key = folderId ?? "";
    let p = paths.get(key);
    if (p === undefined) {
      p = folderPath(index, folderId);
      paths.set(key, p);
    }
    return p;
  };
  const hits = sessions.filter((s) => {
    const text = searchText(s, pathOf(s.folderId));
    const tags = s.tags.map((t) => t.toLowerCase());
    return terms.every((term) =>
      term.startsWith("tag:") && term.length > 4
        ? tags.some((t) => t.includes(term.slice(4)))
        : text.includes(term),
    );
  });
  return hits.sort(
    (a, b) =>
      pathOf(a.folderId).localeCompare(pathOf(b.folderId)) ||
      a.sortOrder - b.sortOrder ||
      a.name.localeCompare(b.name, undefined, { numeric: true }),
  );
}

/** Parse "prod, db  ,web" into ["prod", "db", "web"]. */
export function parseTags(raw: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const part of raw.split(",")) {
    const tag = part.trim();
    if (tag && !seen.has(tag.toLowerCase())) {
      seen.add(tag.toLowerCase());
      out.push(tag);
    }
  }
  return out;
}

/** Colour presets offered in the editor (readable on dark and light themes). */
export const SESSION_COLORS = [
  "#ef4444",
  "#f97316",
  "#eab308",
  "#22c55e",
  "#14b8a6",
  "#3b82f6",
  "#8b5cf6",
  "#ec4899",
] as const;
