/**
 * Open terminals, their SSH sessions, split layouts and multi-exec.
 *
 * Each entry in `tabs` is one terminal ("pane") and owns:
 * - a `TerminalController` (xterm instance + scrollback, survives reconnects),
 * - a `connectionId`: opaque handle to the connection params kept in Rust.
 *   The password never enters this store; Rust uses the handle for
 *   Reconnect/Duplicate and wipes it when the pane closes (`sshForget`),
 * - the current `sessionId` (changes on every reconnect).
 *
 * A tab in the tab bar is a terminal with `parentId === null`. Splitting it
 * adds terminals whose `parentId` is that tab, and `layouts[tabId]` holds the
 * split tree (see `lib/splits`).
 */
import { create } from "zustand";

import {
  IpcError,
  sshConnect,
  sshConnectSaved,
  sshDisconnect,
  sshDuplicate,
  localOpen,
  sshForget,
  sshReconnect,
  type SshSessionCallbacks,
} from "@/lib/ipc";
import {
  MAX_PANES,
  leaf,
  leaves,
  removeLeaf,
  setRatio,
  splitLeaf,
  type SplitDirection,
  type SplitNode,
  type SplitPath,
} from "@/lib/splits";
import { DEFAULT_COLS, DEFAULT_ROWS, TerminalController, type UserInput } from "@/lib/terminal";
import { usePasteStore } from "@/stores/paste";
import { usePromptStore } from "@/stores/prompts";
import type { AppErrorCode, SessionStatus, SshSessionInfo } from "@/types/ipc";
import type { LocalOptions, Protocol, SavedSession } from "@/types/library";

export type TabState = "connecting" | "connected" | "closed";

export interface SessionTab {
  /** Stable client-side id (survives reconnects). */
  id: string;
  /** `null` for a tab in the tab bar; else the tab this split pane belongs to. */
  parentId: string | null;
  /** User-chosen name; `null` means "username@host". */
  customTitle: string | null;
  /** Accent colour from the saved session (`#rrggbb`), if any. */
  color: string | null;
  /** Saved session this tab was opened from, if any. */
  savedSessionId: string | null;
  /** SSH, Telnet, serial line or local shell. */
  protocol: Protocol;
  host: string;
  port: number;
  username: string;
  connectionId: string | null;
  sessionId: string | null;
  state: TabState;
  closedReason: string | null;
  /** Show the live resource bar (saved-session setting; can be hidden per pane). */
  resourceMonitor: boolean;
  controller: TerminalController;
}

export interface ConnectForm {
  host: string;
  port: number;
  username: string;
  password: string;
}

/** Optional presentation details when connecting from a saved session. */
export interface ConnectMeta {
  title?: string | null;
  color?: string | null;
  savedSessionId?: string | null;
}

export const HOME_TAB = "home";

/** Ids of non-terminal tabs (file managers) that may become active. */
const externalTabs = new Set<string>();

export function registerExternalTab(id: string): void {
  externalTabs.add(id);
}

export function unregisterExternalTab(id: string): void {
  externalTabs.delete(id);
}

interface SessionState {
  tabs: SessionTab[];
  /** `HOME_TAB` or a tab id (never a split pane id). */
  activeTabId: string;
  /** Quick-connect dialog is waiting for a login. */
  connecting: boolean;
  /** Terminals whose SFTP side panel is open. */
  sidePanels: string[];
  /** Split tree per tab (only tabs that are split). */
  layouts: Record<string, SplitNode>;
  /** Focused pane per tab. */
  focused: Record<string, string>;
  /** Terminals in multi-exec (empty = off). */
  multiExec: string[];

  toggleSidePanel: (tabId: string) => void;
  setActiveTab: (id: string) => void;
  /** New tab. Resolves on success; throws `IpcError` (user-facing message) on failure. */
  connect: (form: ConnectForm, meta?: ConnectMeta) => Promise<void>;
  /**
   * New tab from a saved session; Rust resolves host and credentials.
   * `extra` carries a username/password the user typed after a
   * CREDENTIALS_REQUIRED answer. Throws `IpcError` on failure.
   */
  connectSaved: (
    session: SavedSession,
    extra?: { username?: string; password?: string },
  ) => Promise<void>;
  /** New tab with a local shell (not a saved session). Throws `IpcError` on failure. */
  openLocal: (options: LocalOptions, title: string) => Promise<void>;
  /** Resolves with the error code if the attempt failed (the tab shows the reason). */
  reconnect: (tabId: string) => Promise<AppErrorCode | null>;
  duplicate: (tabId: string) => Promise<AppErrorCode | null>;
  /**
   * Split a pane: a new terminal on the same server (same login) appears
   * to the right (`row`) or below (`column`).
   */
  splitPane: (paneId: string, direction: SplitDirection) => Promise<AppErrorCode | null>;
  /** Close one pane; closing the last pane closes the tab. */
  closePane: (paneId: string) => Promise<void>;
  focusPane: (paneId: string) => void;
  setSplitRatio: (tabId: string, path: SplitPath, ratio: number) => void;
  /** Start (ids) or stop (`[]`) multi-exec. */
  setMultiExec: (paneIds: string[]) => void;
  setResourceMonitor: (paneId: string, on: boolean) => void;
  disconnect: (tabId: string) => Promise<void>;
  /** Close a tab with all its panes (a pane id closes just that pane). */
  closeTab: (tabId: string) => Promise<void>;
  closeOtherTabs: (tabId: string) => Promise<void>;
  renameTab: (tabId: string, title: string | null) => void;
  /** Move `tabId` to the position currently held by `targetId`. */
  moveTab: (tabId: string, targetId: string) => void;
}

export function tabTitle(tab: SessionTab): string {
  return tab.customTitle ?? (tab.username ? `${tab.username}@${tab.host}` : tab.host);
}

/** Where a terminal is connected: `user@host:port`, `COM3 · 115200 8N1`, `PowerShell`. */
export function tabTarget(
  tab: Pick<SessionTab, "protocol" | "username" | "host" | "port">,
): string {
  if (tab.protocol === "serial" || tab.protocol === "local") return tab.host;
  return `${tab.username ? `${tab.username}@` : ""}${tab.host}:${String(tab.port)}`;
}

/** The tab (in the tab bar) a terminal belongs to. */
export function rootId(tab: SessionTab): string {
  return tab.parentId ?? tab.id;
}

/** Terminals of a tab in layout order (just the tab if it isn't split). */
export function panesOf(
  state: Pick<SessionState, "tabs" | "layouts">,
  tabId: string,
): SessionTab[] {
  const layout = state.layouts[tabId];
  const ids = layout ? leaves(layout) : [tabId];
  return ids.flatMap((id) => state.tabs.filter((t) => t.id === id));
}

/** The terminal keyboard input goes to in the active tab. */
export function activePane(
  state: Pick<SessionState, "tabs" | "activeTabId" | "focused">,
): SessionTab | undefined {
  const id = state.focused[state.activeTabId] ?? state.activeTabId;
  return state.tabs.find((t) => t.id === id) ?? state.tabs.find((t) => t.id === state.activeTabId);
}

const MAX_TITLE_LENGTH = 64;

/** Copy of `record` without `key`. */
function omit<T>(record: Record<string, T>, key: string): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([k]) => k !== key));
}

export const useSessionStore = create<SessionState>()((set, get) => {
  const findTab = (id: string) => get().tabs.find((t) => t.id === id);

  const patchTab = (id: string, patch: Partial<SessionTab>) => {
    set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, ...patch } : t)) }));
  };

  /** Multi-exec: copy what the user typed in `fromId` to the other members. */
  const fanOut = (fromId: string, input: UserInput) => {
    const { multiExec, tabs } = get();
    if (!multiExec.includes(fromId)) return;
    for (const id of multiExec) {
      if (id === fromId) continue;
      const target = tabs.find((t) => t.id === id);
      if (!target || target.state !== "connected") continue;
      if (input.kind === "key") target.controller.sendKey(input.data);
      else target.controller.pasteReviewed(input.text);
    }
  };

  const newController = (id: string): TerminalController => {
    const controller = new TerminalController();
    controller.onUserInput = (input) => {
      fanOut(id, input);
    };
    controller.pasteGuard = (text) => {
      const { multiExec, tabs } = get();
      const targets = multiExec.includes(id)
        ? multiExec.filter((m) => tabs.some((t) => t.id === m && t.state === "connected")).length
        : 1;
      return usePasteStore.getState().review(text, Math.max(1, targets));
    };
    return controller;
  };

  /**
   * Callbacks for one session attempt of one tab. Output always goes to the
   * tab's terminal; a "closed" status is applied only if it belongs to the
   * session the tab currently shows (a late status from a previous session
   * must not mark a fresh reconnect as closed).
   */
  const sessionCallbacks = (tabId: string, controller: TerminalController) => {
    const attempt = { sessionId: null as string | null, earlyClose: null as string | null };
    const onClosed = (reason: string) => {
      const tab = findTab(tabId);
      if (!tab || tab.sessionId !== attempt.sessionId) return;
      controller.detachSession();
      controller.notice(reason);
      patchTab(tabId, { state: "closed", closedReason: reason, sessionId: null });
    };
    const callbacks: SshSessionCallbacks = {
      onOutput: (bytes) => {
        controller.term.write(bytes);
      },
      onStatus: (status: SessionStatus) => {
        if (attempt.sessionId === null) {
          attempt.earlyClose = status.reason; // closed before connect() resolved
        } else {
          onClosed(status.reason);
        }
      },
      onPrompt: (prompt) => {
        usePromptStore.getState().push(prompt);
      },
    };
    /** Call once the backend returned the session id. */
    const started = (info: SshSessionInfo) => {
      attempt.sessionId = info.sessionId;
      controller.attachSession(info.sessionId);
      patchTab(tabId, {
        sessionId: info.sessionId,
        connectionId: info.connectionId,
        state: "connected",
        closedReason: null,
      });
      if (attempt.earlyClose !== null) onClosed(attempt.earlyClose);
    };
    return { callbacks, started };
  };

  const failTab = (tabId: string, controller: TerminalController, err: unknown): AppErrorCode => {
    const reason = err instanceof IpcError ? err.message : "Could not connect.";
    controller.detachSession();
    controller.notice(reason.split("\n")[0] ?? reason);
    patchTab(tabId, { state: "closed", closedReason: reason, sessionId: null });
    return err instanceof IpcError ? err.code : "INTERNAL";
  };

  /** Release a terminal's resources (after it left the store). */
  const release = async (tab: SessionTab) => {
    tab.controller.dispose();
    if (tab.sessionId) await sshDisconnect(tab.sessionId).catch(() => undefined);
    if (tab.connectionId) await sshForget(tab.connectionId).catch(() => undefined);
  };

  /** Drop per-terminal state (multi-exec, side panel) of removed ids. */
  const forgetIds = (ids: string[]) => {
    set((s) => {
      const multiExec = s.multiExec.filter((m) => !ids.includes(m));
      return {
        // Fewer than two left: multi-exec ends on its own.
        multiExec: multiExec.length >= 2 ? multiExec : [],
        sidePanels: s.sidePanels.filter((p) => !ids.includes(p)),
      };
    });
  };

  /**
   * Open a second session on `source`'s connection, placed by `place` (which
   * inserts the new terminal into the store). Shared by Duplicate and Split.
   */
  const spawnFrom = async (
    source: SessionTab,
    place: (tab: SessionTab) => void,
    parentId: string | null,
  ): Promise<AppErrorCode | null> => {
    if (!source.connectionId) return null;
    const id = crypto.randomUUID();
    const controller = newController(id);
    place({
      id,
      parentId,
      customTitle: source.customTitle,
      color: source.color,
      savedSessionId: source.savedSessionId,
      protocol: source.protocol,
      host: source.host,
      port: source.port,
      username: source.username,
      connectionId: null,
      sessionId: null,
      state: "connecting",
      closedReason: null,
      resourceMonitor: source.resourceMonitor,
      controller,
    });
    const { callbacks, started } = sessionCallbacks(id, controller);
    try {
      const { cols, rows } = source.controller.size;
      const info = await sshDuplicate(source.connectionId, cols, rows, callbacks);
      if (!findTab(id)) {
        void sshDisconnect(info.sessionId).catch(() => undefined);
        void sshForget(info.connectionId).catch(() => undefined);
        return null;
      }
      started(info);
      return null;
    } catch (err) {
      return findTab(id) ? failTab(id, controller, err) : null;
    }
  };

  const addTab = (tab: SessionTab) => {
    set((s) => ({ tabs: [...s.tabs, tab], activeTabId: tab.id }));
  };

  return {
    tabs: [],
    activeTabId: HOME_TAB,
    sidePanels: [],
    layouts: {},
    focused: {},
    multiExec: [],
    connecting: false,

    toggleSidePanel: (tabId) => {
      set((st) => ({
        sidePanels: st.sidePanels.includes(tabId)
          ? st.sidePanels.filter((id) => id !== tabId)
          : [...st.sidePanels, tabId],
      }));
    },

    setActiveTab: (id) => {
      if (id === HOME_TAB || externalTabs.has(id)) {
        set({ activeTabId: id });
        return;
      }
      const tab = findTab(id);
      if (!tab) return;
      // A split pane id activates its tab and focuses the pane.
      const root = rootId(tab);
      set((s) => ({
        activeTabId: root,
        focused: tab.parentId ? { ...s.focused, [root]: tab.id } : s.focused,
      }));
    },

    connect: async ({ host, port, username, password }, meta = {}) => {
      set({ connecting: true });
      const id = crypto.randomUUID();
      const controller = newController(id);
      const { callbacks, started } = sessionCallbacks(id, controller);
      try {
        const info = await sshConnect(
          // Rough initial size; the real size is sent as soon as the view mounts.
          { host, port, username, password, cols: DEFAULT_COLS, rows: DEFAULT_ROWS },
          callbacks,
        );
        addTab({
          id,
          parentId: null,
          customTitle: meta.title ?? null,
          color: meta.color ?? null,
          savedSessionId: meta.savedSessionId ?? null,
          protocol: "ssh",
          host,
          port,
          username,
          connectionId: info.connectionId,
          sessionId: null,
          state: "connecting",
          closedReason: null,
          resourceMonitor: true,
          controller,
        });
        started(info);
      } catch (err) {
        controller.dispose();
        throw err;
      } finally {
        set({ connecting: false });
      }
    },

    connectSaved: async (session, extra = {}) => {
      set({ connecting: true });
      const id = crypto.randomUUID();
      const controller = newController(id);
      const { callbacks, started } = sessionCallbacks(id, controller);
      try {
        const info = await sshConnectSaved(
          {
            sessionId: session.id,
            username: extra.username,
            password: extra.password,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
          },
          callbacks,
        );
        addTab({
          id,
          parentId: null,
          customTitle: session.name,
          color: session.color,
          savedSessionId: session.id,
          protocol: session.protocol,
          host: session.host,
          port: session.port,
          username: extra.username ?? session.username,
          connectionId: info.connectionId,
          sessionId: null,
          state: "connecting",
          closedReason: null,
          // The resource bar reads Linux servers over SSH only.
          resourceMonitor: session.protocol === "ssh" && session.resourceMonitor,
          controller,
        });
        started(info);
      } catch (err) {
        controller.dispose();
        throw err;
      } finally {
        set({ connecting: false });
      }
    },

    openLocal: async (options, title) => {
      set({ connecting: true });
      const id = crypto.randomUUID();
      const controller = newController(id);
      const { callbacks, started } = sessionCallbacks(id, controller);
      try {
        const info = await localOpen(options, DEFAULT_COLS, DEFAULT_ROWS, callbacks);
        addTab({
          id,
          parentId: null,
          customTitle: title,
          color: null,
          savedSessionId: null,
          protocol: "local",
          host: title,
          port: 0,
          username: "",
          connectionId: info.connectionId,
          sessionId: null,
          state: "connecting",
          closedReason: null,
          resourceMonitor: false,
          controller,
        });
        started(info);
      } catch (err) {
        controller.dispose();
        throw err;
      } finally {
        set({ connecting: false });
      }
    },

    reconnect: async (tabId) => {
      const tab = findTab(tabId);
      if (!tab || tab.state !== "closed" || !tab.connectionId) return null;
      const { controller, connectionId } = tab;
      patchTab(tabId, { state: "connecting", closedReason: null });
      controller.notice(
        tab.protocol === "local" ? `Restarting ${tab.host}…` : `Reconnecting to ${tabTarget(tab)}…`,
      );
      const { callbacks, started } = sessionCallbacks(tabId, controller);
      try {
        const { cols, rows } = controller.size;
        const info = await sshReconnect(connectionId, cols, rows, callbacks);
        if (!findTab(tabId)) {
          // Tab was closed while reconnecting.
          void sshDisconnect(info.sessionId).catch(() => undefined);
          return null;
        }
        controller.term.write("\r\n");
        started(info);
        return null;
      } catch (err) {
        return findTab(tabId) ? failTab(tabId, controller, err) : null;
      }
    },

    duplicate: async (tabId) => {
      const source = findTab(tabId);
      if (!source) return null;
      // A new tab right after the source's tab, switched to.
      const anchor = rootId(source);
      return spawnFrom(
        source,
        (tab) => {
          set((s) => {
            const tabs = [...s.tabs];
            tabs.splice(tabs.findIndex((t) => t.id === anchor) + 1, 0, tab);
            return { tabs, activeTabId: tab.id };
          });
        },
        null,
      );
    },

    splitPane: async (paneId, direction) => {
      const source = findTab(paneId);
      if (!source?.connectionId) return null;
      const root = rootId(source);
      const current = get().layouts[root] ?? leaf(root);
      if (leaves(current).length >= MAX_PANES) return null;
      return spawnFrom(
        source,
        (tab) => {
          set((s) => ({
            tabs: [...s.tabs, tab],
            layouts: { ...s.layouts, [root]: splitLeaf(current, paneId, tab.id, direction) },
            focused: { ...s.focused, [root]: tab.id },
            activeTabId: root,
          }));
        },
        root,
      );
    },

    closePane: async (paneId) => {
      const tab = findTab(paneId);
      if (!tab) return;
      const root = rootId(tab);
      const layout = get().layouts[root];
      const rest = layout ? removeLeaf(layout, paneId) : null;
      if (!layout || !rest) {
        await get().closeTab(root); // last pane: the whole tab goes
        return;
      }
      const remaining = leaves(rest);
      const nextLayouts = omit(get().layouts, root);
      const nextFocused = omit(get().focused, root);

      if (tab.parentId) {
        if (remaining.length > 1) nextLayouts[root] = rest;
        const wasFocused = get().focused[root] === paneId;
        nextFocused[root] = wasFocused ? (remaining[0] ?? root) : (get().focused[root] ?? root);
        set((s) => ({
          tabs: s.tabs.filter((t) => t.id !== paneId),
          layouts: nextLayouts,
          focused: nextFocused,
        }));
      } else {
        // Closing the tab's own terminal: the first remaining pane takes over
        // the tab (same place in the tab bar, keeps the title).
        const heir = remaining[0];
        if (!heir) return;
        if (remaining.length > 1) nextLayouts[heir] = rest;
        const focusedNow = get().focused[root];
        nextFocused[heir] = focusedNow && focusedNow !== paneId ? focusedNow : heir;
        set((s) => {
          const tabs = s.tabs
            .filter((t) => t.id !== paneId)
            .map((t) =>
              t.id === heir
                ? { ...t, parentId: null, customTitle: tab.customTitle ?? t.customTitle }
                : t.parentId === root
                  ? { ...t, parentId: heir }
                  : t,
            );
          // Put the heir where the old tab was.
          const heirTab = tabs.find((t) => t.id === heir);
          const without = tabs.filter((t) => t.id !== heir);
          const at = s.tabs.findIndex((t) => t.id === paneId);
          if (heirTab) without.splice(Math.min(at, without.length), 0, heirTab);
          return {
            tabs: without,
            layouts: nextLayouts,
            focused: nextFocused,
            activeTabId: s.activeTabId === root ? heir : s.activeTabId,
          };
        });
      }
      forgetIds([paneId]);
      await release(tab);
    },

    focusPane: (paneId) => {
      const tab = findTab(paneId);
      if (!tab) return;
      const root = rootId(tab);
      if (get().focused[root] === paneId) return;
      set((s) => ({ focused: { ...s.focused, [root]: paneId } }));
    },

    setSplitRatio: (tabId, path, ratio) => {
      const layout = get().layouts[tabId];
      if (!layout) return;
      set((s) => ({ layouts: { ...s.layouts, [tabId]: setRatio(layout, path, ratio) } }));
    },

    setMultiExec: (paneIds) => {
      const known = new Set(get().tabs.map((t) => t.id));
      const ids = [...new Set(paneIds)].filter((id) => known.has(id));
      set({ multiExec: ids.length >= 2 ? ids : [] });
    },

    setResourceMonitor: (paneId, on) => {
      patchTab(paneId, { resourceMonitor: on });
    },

    disconnect: async (tabId) => {
      const tab = findTab(tabId);
      if (tab?.sessionId) await sshDisconnect(tab.sessionId).catch(() => undefined);
    },

    closeTab: async (tabId) => {
      const target = findTab(tabId);
      if (!target) return;
      if (target.parentId) {
        await get().closePane(tabId);
        return;
      }
      const { tabs, activeTabId, layouts } = get();
      const group = layouts[tabId] ? leaves(layouts[tabId]) : [tabId];
      const roots = tabs.filter((t) => t.parentId === null);
      const index = roots.findIndex((t) => t.id === tabId);

      // Pick the neighbour to the right, else the left, else Home.
      let nextActive = activeTabId;
      if (activeTabId === tabId) {
        const neighbour = roots[index + 1] ?? roots[index - 1];
        nextActive = neighbour ? neighbour.id : HOME_TAB;
      }
      const closing = tabs.filter((t) => group.includes(t.id) || t.parentId === tabId);
      const closingIds = closing.map((t) => t.id);
      set((s) => {
        const nextLayouts = omit(s.layouts, tabId);
        const nextFocused = omit(s.focused, tabId);
        return {
          tabs: s.tabs.filter((t) => !closingIds.includes(t.id)),
          activeTabId: nextActive,
          layouts: nextLayouts,
          focused: nextFocused,
        };
      });
      forgetIds(closingIds);
      await Promise.all(closing.map(release));
    },

    closeOtherTabs: async (tabId) => {
      const target = findTab(tabId);
      const keep = target ? rootId(target) : tabId;
      const others = get().tabs.filter((t) => t.parentId === null && t.id !== keep);
      await Promise.all(others.map((t) => get().closeTab(t.id)));
    },

    renameTab: (tabId, title) => {
      const trimmed = title?.trim().slice(0, MAX_TITLE_LENGTH) ?? "";
      patchTab(tabId, { customTitle: trimmed === "" ? null : trimmed });
    },

    moveTab: (tabId, targetId) => {
      if (tabId === targetId) return;
      set((s) => {
        const from = s.tabs.findIndex((t) => t.id === tabId);
        const to = s.tabs.findIndex((t) => t.id === targetId);
        if (from < 0 || to < 0) return s;
        const tabs = [...s.tabs];
        const [moved] = tabs.splice(from, 1);
        if (!moved) return s;
        tabs.splice(to, 0, moved);
        return { tabs };
      });
    },
  };
});
