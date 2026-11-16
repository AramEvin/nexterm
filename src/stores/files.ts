/**
 * File-manager tabs. Each tab has two panes; a pane shows the local disk or
 * a server (SFTP). Remote panes hold only an opaque `fsId`: the connection
 * and its credentials live in Rust.
 */
import { create } from "zustand";

import { handleConnectError } from "@/lib/connect";
import { IpcError, fsList, sftpClose, sftpOpenConnection, sftpOpenSaved, fsHome } from "@/lib/ipc";
import { useDialogStore } from "@/stores/dialogs";
import { usePromptStore } from "@/stores/prompts";
import {
  HOME_TAB,
  registerExternalTab,
  unregisterExternalTab,
  useSessionStore,
} from "@/stores/session";
import type { Endpoint, Listing } from "@/types/files";

/** Where a pane points. */
export type PaneSource =
  | { kind: "local" }
  /** A saved session (Rust resolves host and credentials). */
  | { kind: "saved"; sessionId: string; name: string; host: string; port: number }
  /** The server of an open terminal tab (reuses its login). */
  | { kind: "connection"; connectionId: string; name: string; host: string; port: number };

export type PaneStatus = "picker" | "connecting" | "ready" | "needsLogin" | "error";

export interface Pane {
  source: PaneSource | null;
  fsId: string | null;
  /** "Local disk" or "user@host:port". */
  label: string;
  status: PaneStatus;
  /** Why the pane can't show files (status error / needsLogin). */
  error: string | null;
  path: string;
  /** Start folder (login folder / user profile). */
  home: string;
  listing: Listing | null;
  loading: boolean;
  /** Last failed action (shown above the list; the listing stays). */
  notice: string | null;
  selected: string[];
  showHidden: boolean;
}

export interface FilesTab {
  id: string;
  panes: [Pane, Pane];
  activePane: 0 | 1;
}

export type PaneIndex = 0 | 1;

function emptyPane(source: PaneSource | null): Pane {
  return {
    source,
    fsId: null,
    label: source ? sourceLabel(source) : "Choose a location",
    status: source ? "connecting" : "picker",
    error: null,
    path: "",
    home: "",
    listing: null,
    loading: false,
    notice: null,
    selected: [],
    showHidden: false,
  };
}

export function sourceLabel(source: PaneSource): string {
  return source.kind === "local" ? "Local disk" : source.name;
}

export function paneEndpoint(pane: Pane): Endpoint | null {
  if (pane.source?.kind === "local") return { kind: "local" };
  if (pane.fsId) return { kind: "remote", fsId: pane.fsId };
  return null;
}

export function isLocal(pane: Pane): boolean {
  return pane.source?.kind === "local";
}

interface FilesState {
  tabs: FilesTab[];
  /** New file-manager tab: local disk on the left, `right` (or a picker) on the right. */
  openTab: (right?: PaneSource) => string;
  closeTab: (id: string) => Promise<void>;
  setActivePane: (tabId: string, pane: PaneIndex) => void;
  /** Point a pane at a source (connects if remote). */
  setSource: (
    tabId: string,
    pane: PaneIndex,
    source: PaneSource | null,
    login?: { username?: string; password?: string },
    /** Folder to open after connecting (reconnect); falls back to home. */
    startPath?: string,
  ) => Promise<void>;
  /** Open a folder. Resolves true on success. */
  navigate: (tabId: string, pane: PaneIndex, path: string) => Promise<boolean>;
  refresh: (tabId: string, pane: PaneIndex) => Promise<void>;
  /** Refresh every pane (in any tab) that shows `dir` on `endpoint`. */
  refreshWhere: (endpoint: Endpoint, dir: string) => void;
  setSelected: (tabId: string, pane: PaneIndex, selected: string[]) => void;
  setNotice: (tabId: string, pane: PaneIndex, notice: string | null) => void;
  toggleHidden: (tabId: string, pane: PaneIndex) => void;
}

/** Same endpoint? (same local disk, or the same SFTP session) */
export function sameEndpoint(a: Endpoint | null, b: Endpoint | null): boolean {
  if (!a || !b) return false;
  if (a.kind === "local" && b.kind === "local") return true;
  return a.kind === "remote" && b.kind === "remote" && a.fsId === b.fsId;
}

export const useFilesStore = create<FilesState>()((set, get) => {
  const getPane = (tabId: string, idx: PaneIndex) =>
    get().tabs.find((t) => t.id === tabId)?.panes[idx];

  const patchPane = (tabId: string, idx: PaneIndex, p: Partial<Pane>) => {
    set((s) => ({
      tabs: s.tabs.map((t) => {
        if (t.id !== tabId) return t;
        const panes: [Pane, Pane] = [t.panes[0], t.panes[1]];
        panes[idx] = { ...panes[idx], ...p };
        return { ...t, panes };
      }),
    }));
  };

  const closeRemote = (pane: Pane | undefined) => {
    if (pane?.fsId) void sftpClose(pane.fsId).catch(() => undefined);
  };

  const onPrompt = (prompt: Parameters<ReturnType<typeof usePromptStore.getState>["push"]>[0]) => {
    usePromptStore.getState().push(prompt);
  };

  return {
    tabs: [],

    openTab: (right) => {
      const id = crypto.randomUUID();
      const tab: FilesTab = {
        id,
        panes: [emptyPane({ kind: "local" }), emptyPane(right ?? null)],
        activePane: right ? 1 : 0,
      };
      set((s) => ({ tabs: [...s.tabs, tab] }));
      registerExternalTab(id);
      useSessionStore.getState().setActiveTab(id);
      void get().setSource(id, 0, { kind: "local" });
      if (right) void get().setSource(id, 1, right);
      return id;
    },

    closeTab: async (id) => {
      const tab = get().tabs.find((t) => t.id === id);
      if (!tab) return;
      tab.panes.forEach(closeRemote);
      set((s) => ({ tabs: s.tabs.filter((t) => t.id !== id) }));
      unregisterExternalTab(id);
      const sessions = useSessionStore.getState();
      if (sessions.activeTabId === id) {
        const remaining = get().tabs;
        const next =
          remaining[remaining.length - 1]?.id ??
          sessions.tabs[sessions.tabs.length - 1]?.id ??
          HOME_TAB;
        sessions.setActiveTab(next);
      }
      await Promise.resolve();
    },

    setActivePane: (tabId, pane) => {
      set((s) => ({ tabs: s.tabs.map((t) => (t.id === tabId ? { ...t, activePane: pane } : t)) }));
    },

    setSource: async (tabId, idx, source, login, startPath) => {
      closeRemote(getPane(tabId, idx));
      const fresh = emptyPane(source);
      patchPane(tabId, idx, { ...fresh, showHidden: getPane(tabId, idx)?.showHidden ?? false });
      if (!source) return;
      if (source.kind === "local") {
        const home = await fsHome({ kind: "local" }).catch(() => "");
        patchPane(tabId, idx, { status: "ready", home });
        if (!startPath || !(await get().navigate(tabId, idx, startPath))) {
          await get().navigate(tabId, idx, home);
        }
        return;
      }
      try {
        const info =
          source.kind === "saved"
            ? await sftpOpenSaved({ sessionId: source.sessionId, ...login }, onPrompt)
            : await sftpOpenConnection(source.connectionId, onPrompt);
        // The pane may have been pointed elsewhere (or closed) meanwhile.
        if (getPane(tabId, idx)?.source !== source) {
          void sftpClose(info.fsId).catch(() => undefined);
          return;
        }
        patchPane(tabId, idx, {
          fsId: info.fsId,
          label: info.label,
          status: "ready",
          home: info.home,
        });
        if (!startPath || !(await get().navigate(tabId, idx, startPath))) {
          await get().navigate(tabId, idx, info.home);
        }
      } catch (err) {
        if (getPane(tabId, idx)?.source !== source) return;
        const { code, message } =
          err instanceof IpcError ? err : { code: "INTERNAL" as const, message: "Failed." };
        if (code === "CREDENTIALS_REQUIRED") {
          // Remember the folder so the login form can reopen it.
          patchPane(tabId, idx, { status: "needsLogin", error: message, path: startPath ?? "" });
          return;
        }
        patchPane(tabId, idx, {
          status: "error",
          error:
            code === "HOST_KEY_REJECTED" || code === "CANCELLED"
              ? "Connection cancelled."
              : code === "VAULT_LOCKED"
                ? "The vault is locked."
                : message,
        });
        handleConnectError(code, message, {
          host: source.host,
          port: source.port,
          retry: () => void get().setSource(tabId, idx, source, login, startPath),
        });
      }
    },

    navigate: async (tabId, idx, path) => {
      const pane = getPane(tabId, idx);
      const endpoint = pane && paneEndpoint(pane);
      if (!pane || !endpoint) return false;
      patchPane(tabId, idx, { loading: true });
      try {
        const listing = await fsList(endpoint, path);
        if (getPane(tabId, idx)?.fsId !== pane.fsId) return false;
        const keep = new Set(listing.entries.map((e) => e.path));
        patchPane(tabId, idx, {
          listing,
          path: listing.path,
          loading: false,
          notice: listing.truncated
            ? `Showing the first ${String(listing.entries.length)} items of this folder.`
            : null,
          // Keep the selection when refreshing the same folder.
          selected: listing.path === pane.path ? pane.selected.filter((p) => keep.has(p)) : [],
        });
        return true;
      } catch (err) {
        const { code, message } =
          err instanceof IpcError ? err : { code: "INTERNAL" as const, message: "Failed." };
        if (code === "CONNECTION" || (code === "NOT_FOUND" && pane.fsId)) {
          patchPane(tabId, idx, {
            loading: false,
            status: "error",
            error: code === "CONNECTION" ? message : "The connection was closed.",
          });
        } else {
          patchPane(tabId, idx, { loading: false, notice: message });
        }
        return false;
      }
    },

    refresh: async (tabId, idx) => {
      const pane = getPane(tabId, idx);
      if (pane?.status === "ready") await get().navigate(tabId, idx, pane.path);
    },

    refreshWhere: (endpoint, dir) => {
      for (const tab of get().tabs) {
        tab.panes.forEach((pane, i) => {
          if (
            pane.status === "ready" &&
            pane.path === dir &&
            sameEndpoint(paneEndpoint(pane), endpoint)
          ) {
            void get().refresh(tab.id, i as PaneIndex);
          }
        });
      }
    },

    setSelected: (tabId, idx, selected) => {
      patchPane(tabId, idx, { selected });
    },

    setNotice: (tabId, idx, notice) => {
      patchPane(tabId, idx, { notice });
    },

    toggleHidden: (tabId, idx) => {
      const pane = getPane(tabId, idx);
      if (pane) patchPane(tabId, idx, { showHidden: !pane.showHidden });
    },
  };
});

/** Open a file-manager tab on a saved session (from the session tree). */
export function openFilesForSaved(session: {
  id: string;
  name: string;
  host: string;
  port: number;
}): void {
  useFilesStore.getState().openTab({
    kind: "saved",
    sessionId: session.id,
    name: session.name,
    host: session.host,
    port: session.port,
  });
}

/** Open a file-manager tab on a terminal tab's server. */
export function openFilesForTerminal(tab: {
  connectionId: string | null;
  host: string;
  port: number;
  username: string;
  customTitle: string | null;
}): void {
  if (!tab.connectionId) {
    useDialogStore.getState().openAlert({
      title: "Not connected",
      message: "Connect this tab first, then open its files.",
    });
    return;
  }
  useFilesStore.getState().openTab({
    kind: "connection",
    connectionId: tab.connectionId,
    name: tab.customTitle ?? `${tab.username}@${tab.host}`,
    host: tab.host,
    port: tab.port,
  });
}
