/** Pure helpers for the file manager (formatting, sorting, selection). */
import { isDirLike, type FileEntry } from "@/types/files";

const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/** 1536 -> "1.5 KB" (binary units, like Explorer). */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "";
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toFixed(digits)} ${UNITS[unit] ?? "B"}`;
}

export function formatSpeed(bytesPerSec: number): string {
  return bytesPerSec > 0 ? `${formatBytes(bytesPerSec)}/s` : "";
}

/** Seconds left -> "2 h 05 min" / "3 min 20 s" / "45 s". */
export function formatEta(remainingBytes: number, bytesPerSec: number): string {
  if (bytesPerSec <= 0 || remainingBytes <= 0) return "";
  const total = Math.ceil(remainingBytes / bytesPerSec);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${String(h)} h ${String(m).padStart(2, "0")} min`;
  if (m > 0) return `${String(m)} min ${String(s).padStart(2, "0")} s`;
  return `${String(s)} s`;
}

/** 0o755 -> "rwxr-xr-x" (setuid/setgid/sticky shown as s/s/t). */
export function formatMode(mode: number | null): string {
  if (mode === null) return "";
  const bits = ["r", "w", "x"];
  let out = "";
  for (let i = 8; i >= 0; i--) {
    out += mode & (1 << i) ? (bits[(8 - i) % 3] ?? "-") : "-";
  }
  const chars = out.split("");
  const special = (idx: number, flag: number, on: string, offChar: string) => {
    if (mode & flag) chars[idx] = chars[idx] === "x" ? on : offChar;
  };
  special(2, 0o4000, "s", "S");
  special(5, 0o2000, "s", "S");
  special(8, 0o1000, "t", "T");
  return chars.join("");
}

/** 0o644 -> "644"; keeps a leading special digit when set. */
export function formatOctal(mode: number): string {
  const s = (mode & 0o7777).toString(8);
  return mode & 0o7000 ? s.padStart(4, "0") : s.padStart(3, "0");
}

/** "644" / "0755" -> number, or null if not valid octal permissions. */
export function parseOctal(text: string): number | null {
  const t = text.trim();
  if (!/^[0-7]{3,4}$/.test(t)) return null;
  return parseInt(t, 8);
}

const dateFormat = new Intl.DateTimeFormat(undefined, {
  year: "2-digit",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
});

export function formatDate(unixSeconds: number | null): string {
  return unixSeconds === null ? "" : dateFormat.format(new Date(unixSeconds * 1000));
}

export type SortKey = "name" | "size" | "modified" | "permissions" | "owner";
export interface SortSpec {
  key: SortKey;
  asc: boolean;
}

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

/** Folders always first; then by the chosen column. */
export function sortEntries(entries: readonly FileEntry[], sort: SortSpec): FileEntry[] {
  const dir = sort.asc ? 1 : -1;
  const byName = (a: FileEntry, b: FileEntry) => collator.compare(a.name, b.name);
  return [...entries].sort((a, b) => {
    const folder = Number(isDirLike(b)) - Number(isDirLike(a));
    if (folder !== 0) return folder;
    let c = 0;
    switch (sort.key) {
      case "size":
        c = a.size - b.size;
        break;
      case "modified":
        c = (a.modified ?? 0) - (b.modified ?? 0);
        break;
      case "permissions":
        c = (a.permissions ?? 0) - (b.permissions ?? 0);
        break;
      case "owner":
        c = collator.compare(a.owner ?? "", b.owner ?? "");
        break;
      case "name":
        break;
    }
    return (c !== 0 ? c : byName(a, b)) * dir;
  });
}

/**
 * Selection after a click on `index` (Explorer rules): plain click selects one,
 * Ctrl toggles, Shift selects the range from the anchor.
 */
export function clickSelection(
  paths: readonly string[],
  selected: ReadonlySet<string>,
  anchor: number | null,
  index: number,
  mods: { ctrl: boolean; shift: boolean },
): { selected: Set<string>; anchor: number } {
  const path = paths[index];
  if (path === undefined) return { selected: new Set(selected), anchor: anchor ?? 0 };
  if (mods.shift && anchor !== null) {
    const [from, to] = anchor < index ? [anchor, index] : [index, anchor];
    const range = new Set(mods.ctrl ? selected : []);
    for (let i = from; i <= to; i++) {
      const p = paths[i];
      if (p !== undefined) range.add(p);
    }
    return { selected: range, anchor };
  }
  if (mods.ctrl) {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    return { selected: next, anchor: index };
  }
  return { selected: new Set([path]), anchor: index };
}

/** Names in `incoming` that already exist in `existing`. */
export function conflictingNames(
  incoming: readonly string[],
  existing: readonly FileEntry[],
  caseInsensitive: boolean,
): string[] {
  const norm = (n: string) => (caseInsensitive ? n.toLowerCase() : n);
  const have = new Set(existing.map((e) => norm(e.name)));
  return incoming.filter((n) => have.has(norm(n)));
}

/** Last path component, either separator. */
export function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const parts = trimmed.split(/[\\/]/);
  return parts[parts.length - 1] || trimmed;
}

/** Breadcrumb segments for a path: [{label, path}]. */
export function breadcrumbs(path: string, local: boolean): { label: string; path: string }[] {
  if (path === "") return [];
  if (!local || path.startsWith("/")) {
    const out = [{ label: "/", path: "/" }];
    let acc = "";
    for (const part of path.split("/").filter(Boolean)) {
      acc += `/${part}`;
      out.push({ label: part, path: acc });
    }
    return out;
  }
  // Windows: "C:\Users\me" -> C:\, Users, me
  const parts = path.split("\\").filter(Boolean);
  const out: { label: string; path: string }[] = [];
  let acc = "";
  parts.forEach((part, i) => {
    acc = i === 0 ? `${part}\\` : `${acc}${i === 1 ? "" : "\\"}${part}`;
    out.push({ label: i === 0 ? `${part}\\` : part, path: acc });
  });
  return out;
}
