/**
 * Open tabs and their SSH sessions.
 *
 * Each tab owns:
 * - a `TerminalController` (xterm instance + scrollback, survives reconnects),
 * - a `connectionId`: opaque handle to the connection params kept in Rust.
 *   The password never enters this store; Rust uses the handle for
 *   Reconnect/Duplicate and wipes it when the tab closes (`sshForget`),
 * - the current `sessionId` (changes on every reconnect).
 */
import { create } from "zustand";

import {
  IpcError,
  sshConnect,
  sshDisconnect,
  sshDuplicate,
  sshForget,
  sshReconnect,
  type SshSessionCallbacks,
} from "@/lib/ipc";
import { DEFAULT_COLS, DEFAULT_ROWS, TerminalController } from "@/lib/terminal";
import type { SessionStatus, SshSessionInfo } from "@/types/ipc";

export type TabState = "connecting" | "connected" | "closed";

export interface SessionTab {
  /** Stable client-side id (survives reconnects). */
  id: string;
  /** User-chosen name; `null` means "username@host". */
  customTitle: string | null;
  host: string;
  port: number;
  username: string;
  connectionId: string | null;
  sessionId: string | null;
  state: TabState;
  closedReason: string | null;
  controller: TerminalController;
}

export interface ConnectForm {
  host: string;
  port: number;
  username: string;
  password: string;
}

export const HOME_TAB = "home";

interface SessionState {
  tabs: SessionTab[];
  /** `HOME_TAB` or a tab id. */
  activeTabId: string;
  /** Quick-connect dialog is waiting for a login. */
  connecting: boolean;

  setActiveTab: (id: string) => void;
  /** New tab. Resolves on success; throws `IpcError` (user-facing message) on failure. */
  connect: (form: ConnectForm) => Promise<void>;
  reconnect: (tabId: string) => Promise<void>;
  duplicate: (tabId: string) => Promise<void>;
  disconnect: (tabId: string) => Promise<void>;
  closeTab: (tabId: string) => Promise<void>;
  closeOtherTabs: (tabId: string) => Promise<void>;
  renameTab: (tabId: string, title: string | null) => void;
  /** Move `tabId` to the position currently held by `targetId`. */
  moveTab: (tabId: string, targetId: string) => void;
}

export function tabTitle(tab: SessionTab): string {
  return tab.customTitle ?? `${tab.username}@${tab.host}`;
}

const MAX_TITLE_LENGTH = 64;

export const useSessionStore = create<SessionState>()((set, get) => {
  const findTab = (id: string) => get().tabs.find((t) => t.id === id);

  const patchTab = (id: string, patch: Partial<SessionTab>) => {
    set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, ...patch } : t)) }));
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

  const failTab = (tabId: string, controller: TerminalController, err: unknown) => {
    const reason = err instanceof IpcError ? err.message : "Could not connect.";
    controller.detachSession();
    controller.notice(reason);
    patchTab(tabId, { state: "closed", closedReason: reason, sessionId: null });
  };

  return {
    tabs: [],
    activeTabId: HOME_TAB,
    connecting: false,

    setActiveTab: (id) => {
      if (id === HOME_TAB || findTab(id)) set({ activeTabId: id });
    },

    connect: async ({ host, port, username, password }) => {
      set({ connecting: true });
      const id = crypto.randomUUID();
      const controller = new TerminalController();
      const { callbacks, started } = sessionCallbacks(id, controller);
      try {
        const info = await sshConnect(
          // Rough initial size; the real size is sent as soon as the view mounts.
          { host, port, username, password, cols: DEFAULT_COLS, rows: DEFAULT_ROWS },
          callbacks,
        );
        set((s) => ({
          tabs: [
            ...s.tabs,
            {
              id,
              customTitle: null,
              host,
              port,
              username,
              connectionId: info.connectionId,
              sessionId: null,
              state: "connecting",
              closedReason: null,
              controller,
            },
          ],
          activeTabId: id,
        }));
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
      if (!tab || tab.state !== "closed" || !tab.connectionId) return;
      const { controller, connectionId } = tab;
      patchTab(tabId, { state: "connecting", closedReason: null });
      controller.notice(`Reconnecting to ${tab.host}:${tab.port}…`);
      const { callbacks, started } = sessionCallbacks(tabId, controller);
      try {
        const { cols, rows } = controller.size;
        const info = await sshReconnect(connectionId, cols, rows, callbacks);
        if (!findTab(tabId)) {
          // Tab was closed while reconnecting.
          void sshDisconnect(info.sessionId).catch(() => undefined);
          return;
        }
        controller.term.write("\r\n");
        started(info);
      } catch (err) {
        if (findTab(tabId)) failTab(tabId, controller, err);
      }
    },

    duplicate: async (tabId) => {
      const source = findTab(tabId);
      if (!source?.connectionId) return;
      const id = crypto.randomUUID();
      const controller = new TerminalController();
      const sourceIndex = get().tabs.findIndex((t) => t.id === tabId);
      const newTab: SessionTab = {
        id,
        customTitle: source.customTitle,
        host: source.host,
        port: source.port,
        username: source.username,
        connectionId: null,
        sessionId: null,
        state: "connecting",
        closedReason: null,
        controller,
      };
      // Insert right after the source tab and switch to it.
      set((s) => {
        const tabs = [...s.tabs];
        tabs.splice(sourceIndex + 1, 0, newTab);
        return { tabs, activeTabId: id };
      });
      const { callbacks, started } = sessionCallbacks(id, controller);
      try {
        const { cols, rows } = source.controller.size;
        const info = await sshDuplicate(source.connectionId, cols, rows, callbacks);
        if (!findTab(id)) {
          void sshDisconnect(info.sessionId).catch(() => undefined);
          void sshForget(info.connectionId).catch(() => undefined);
          return;
        }
        started(info);
      } catch (err) {
        if (findTab(id)) failTab(id, controller, err);
      }
    },

    disconnect: async (tabId) => {
      const tab = findTab(tabId);
      if (tab?.sessionId) await sshDisconnect(tab.sessionId).catch(() => undefined);
    },

    closeTab: async (tabId) => {
      const { tabs, activeTabId } = get();
      const index = tabs.findIndex((t) => t.id === tabId);
      const tab = tabs[index];
      if (!tab) return;

      // Pick the neighbour to the right, else the left, else Home.
      let nextActive = activeTabId;
      if (activeTabId === tabId) {
        const neighbour = tabs[index + 1] ?? tabs[index - 1];
        nextActive = neighbour ? neighbour.id : HOME_TAB;
      }
      set({ tabs: tabs.filter((t) => t.id !== tabId), activeTabId: nextActive });

      tab.controller.dispose();
      if (tab.sessionId) await sshDisconnect(tab.sessionId).catch(() => undefined);
      if (tab.connectionId) await sshForget(tab.connectionId).catch(() => undefined);
    },

    closeOtherTabs: async (tabId) => {
      const others = get().tabs.filter((t) => t.id !== tabId);
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
