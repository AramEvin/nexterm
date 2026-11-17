/**
 * File-manager actions shared by toolbars, context menus, keyboard shortcuts
 * and drag & drop, so every entry point behaves the same.
 */
import { baseName, conflictingNames } from "@/lib/files";
import {
  fsChmod,
  fsDelete,
  fsMkdir,
  fsRename,
  IpcError,
  pickLocalFiles,
  pickLocalFolder,
} from "@/lib/ipc";
import { useDialogStore } from "@/stores/dialogs";
import { chooseConflict, useFileDialogStore } from "@/stores/fileDialogs";
import { isLocal, paneEndpoint, useFilesStore, type Pane, type PaneIndex } from "@/stores/files";
import { useEditStore } from "@/stores/edits";
import { useTransferStore } from "@/stores/transfers";
import type { Conflict, Endpoint, FileEntry } from "@/types/files";

const isWindows = typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);

function pane(tabId: string, idx: PaneIndex): Pane | undefined {
  return useFilesStore.getState().tabs.find((t) => t.id === tabId)?.panes[idx];
}

function alert(title: string, message: string) {
  useDialogStore.getState().openAlert({ title, message });
}

function errorMessage(err: unknown, fallback: string): string {
  return err instanceof IpcError ? err.message : fallback;
}

/** "3 items" / "notes.txt" */
export function describeItems(paths: readonly string[]): string {
  return paths.length === 1 ? `"${baseName(paths[0] ?? "")}"` : `${String(paths.length)} items`;
}

interface CopySide {
  endpoint: Endpoint;
  label: string;
  dir: string;
}

/**
 * Copy `paths` from `source` into `target.dir`. `visible` = the target
 * folder's listing when it is on screen: then name clashes are asked about;
 * otherwise a clash keeps both copies (never overwrites blindly).
 */
async function startCopy(
  source: CopySide,
  paths: string[],
  target: CopySide,
  visible: { entries: FileEntry[]; caseInsensitive: boolean } | null,
): Promise<void> {
  const names = paths.map(baseName);
  let conflict: Conflict = "rename";
  if (visible) {
    const clashes = conflictingNames(names, visible.entries, visible.caseInsensitive);
    if (clashes.length > 0) {
      const choice = await chooseConflict(clashes, `${target.label}: ${target.dir}`);
      if (!choice) return;
      conflict = choice;
    } else {
      conflict = "overwrite";
    }
  }
  await useTransferStore.getState().start(
    {
      source: source.endpoint,
      sources: paths,
      target: target.endpoint,
      targetDir: target.dir,
      conflict,
    },
    {
      title: paths.length === 1 ? (names[0] ?? "") : `${String(paths.length)} items`,
      from: `${source.label}: ${source.dir}`,
      to: `${target.label}: ${target.dir}`,
      onFinished: () => {
        useFilesStore.getState().refreshWhere(target.endpoint, target.dir);
      },
    },
  );
}

/** Copy into pane `targetIdx` (its current folder, or `targetDir`). */
async function queueCopy(
  tabId: string,
  source: CopySide,
  paths: string[],
  targetIdx: PaneIndex,
  targetDir?: string,
): Promise<void> {
  const target = pane(tabId, targetIdx);
  const endpoint = target && paneEndpoint(target);
  if (!target || !endpoint || target.status !== "ready") {
    alert("Nowhere to copy to", "Connect the other side first.");
    return;
  }
  const dir = targetDir ?? target.path;
  if (dir === "") {
    alert("Open a folder first", "Choose a drive or folder on the other side to copy into.");
    return;
  }
  const visible =
    dir === target.path && target.listing
      ? { entries: target.listing.entries, caseInsensitive: isLocal(target) && isWindows }
      : null;
  await startCopy(source, paths, { endpoint, label: target.label, dir }, visible);
}

/** Side panel: download the selection into a folder chosen in a native dialog. */
export async function downloadTo(tabId: string, idx: PaneIndex, paths: string[]): Promise<void> {
  const src = pane(tabId, idx);
  const endpoint = src && paneEndpoint(src);
  if (!src || !endpoint || paths.length === 0) return;
  const dir = await pickLocalFolder().catch(() => null);
  if (!dir) return;
  await startCopy(
    { endpoint, label: src.label, dir: src.path },
    paths,
    { endpoint: { kind: "local" }, label: "Local disk", dir },
    null,
  );
}

/** Side panel: upload files chosen in a native dialog into the pane's folder. */
export async function uploadFiles(tabId: string, idx: PaneIndex): Promise<void> {
  const paths = await pickLocalFiles().catch(() => [] as string[]);
  if (paths.length > 0) await copyLocalPathsInto(tabId, idx, paths);
}

/** Open a remote file in the local editor (Notepad); saves go back to the server. */
export function editItem(tabId: string, idx: PaneIndex, path: string): void {
  const p = pane(tabId, idx);
  const endpoint = p && paneEndpoint(p);
  if (!p || !endpoint || endpoint.kind === "local") return;
  void useEditStore.getState().start(endpoint, path, p.label);
}

/** F5 / "Copy to other side" / drag between panes. */
export async function copyToOtherPane(
  tabId: string,
  from: PaneIndex,
  paths: string[],
  targetDir?: string,
): Promise<void> {
  const src = pane(tabId, from);
  const endpoint = src && paneEndpoint(src);
  if (!src || !endpoint || paths.length === 0) return;
  await queueCopy(
    tabId,
    { endpoint, label: src.label, dir: src.path },
    paths,
    from === 0 ? 1 : 0,
    targetDir,
  );
}

/** Files dropped from Windows Explorer onto a pane. */
export async function copyLocalPathsInto(
  tabId: string,
  targetIdx: PaneIndex,
  paths: string[],
  targetDir?: string,
): Promise<void> {
  if (paths.length === 0) return;
  const first = paths[0] ?? "";
  const dir = first.slice(0, Math.max(0, first.length - baseName(first).length));
  await queueCopy(
    tabId,
    { endpoint: { kind: "local" }, label: "Local disk", dir },
    paths,
    targetIdx,
    targetDir,
  );
}

export function newFolder(tabId: string, idx: PaneIndex): void {
  const p = pane(tabId, idx);
  const endpoint = p && paneEndpoint(p);
  if (!p || !endpoint || p.path === "") return;
  useFileDialogStore.getState().askName({
    title: "New folder",
    label: "Folder name",
    initial: "New folder",
    confirmLabel: "Create",
    onSubmit: async (name) => {
      await fsMkdir(endpoint, p.path, name);
      await useFilesStore.getState().refresh(tabId, idx);
    },
  });
}

export function renameItem(tabId: string, idx: PaneIndex, path: string): void {
  const p = pane(tabId, idx);
  const endpoint = p && paneEndpoint(p);
  if (!p || !endpoint) return;
  useFileDialogStore.getState().askName({
    title: "Rename",
    label: "New name",
    initial: baseName(path),
    confirmLabel: "Rename",
    onSubmit: async (name) => {
      if (name === baseName(path)) return;
      const renamed = await fsRename(endpoint, path, name);
      await useFilesStore.getState().refresh(tabId, idx);
      useFilesStore.getState().setSelected(tabId, idx, [renamed]);
    },
  });
}

export function deleteItems(tabId: string, idx: PaneIndex, paths: string[]): void {
  const p = pane(tabId, idx);
  const endpoint = p && paneEndpoint(p);
  if (!p || !endpoint || paths.length === 0) return;
  const what = describeItems(paths);
  useDialogStore.getState().askConfirm({
    title: `Delete ${what}?`,
    description: `${what} will be permanently deleted from ${p.label} (folders with everything inside). This can't be undone.`,
    confirmLabel: "Delete",
    destructive: true,
    onConfirm: async () => {
      try {
        await fsDelete(endpoint, paths);
      } finally {
        await useFilesStore.getState().refresh(tabId, idx);
      }
    },
  });
}

export function changePermissions(tabId: string, idx: PaneIndex, paths: string[]): void {
  const p = pane(tabId, idx);
  const endpoint = p && paneEndpoint(p);
  if (!p || !endpoint || paths.length === 0) return;
  const first = p.listing?.entries.find((e) => e.path === paths[0]);
  useFileDialogStore.getState().askChmod({
    what: describeItems(paths),
    initialMode: first?.permissions ?? 0o644,
    onSubmit: async (mode) => {
      await fsChmod(endpoint, paths, mode);
      await useFilesStore.getState().refresh(tabId, idx);
    },
  });
}

export async function copyPathToClipboard(paths: string[]): Promise<void> {
  try {
    await navigator.clipboard.writeText(paths.join("\n"));
  } catch (err) {
    alert("Copy failed", errorMessage(err, "The clipboard is not available."));
  }
}
