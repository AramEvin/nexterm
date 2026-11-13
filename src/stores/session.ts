/**
 * The live SSH session (Phase 1: one at a time; Phase 2 makes this a list).
 *
 * The password is passed straight through to `sshConnect` and never stored
 * in this store, so it can't linger in React state or devtools.
 */
import { create } from "zustand";

import { IpcError, sshConnect, sshDisconnect } from "@/lib/ipc";
import { TerminalController } from "@/lib/terminal";

export type ConnectionState = "connected" | "closed";

export interface LiveSession {
  id: string;
  host: string;
  port: number;
  username: string;
  state: ConnectionState;
  closedReason: string | null;
  controller: TerminalController;
}

export interface ConnectForm {
  host: string;
  port: number;
  username: string;
  password: string;
}

export type TabKey = "home" | "session";

interface SessionState {
  session: LiveSession | null;
  activeTab: TabKey;
  connecting: boolean;
  setActiveTab: (tab: TabKey) => void;
  /** Resolves on success; throws `IpcError` with a user-facing message on failure. */
  connect: (form: ConnectForm) => Promise<void>;
  disconnect: () => Promise<void>;
  closeTab: () => Promise<void>;
}

export const useSessionStore = create<SessionState>()((set, get) => ({
  session: null,
  activeTab: "home",
  connecting: false,

  setActiveTab: (tab) => {
    set({ activeTab: tab });
  },

  connect: async ({ host, port, username, password }) => {
    if (get().session) {
      throw new IpcError("VALIDATION", "Close the current session first.");
    }
    set({ connecting: true });
    const controller = new TerminalController();

    try {
      const id = await sshConnect(
        // Rough initial size; the real size is sent as soon as the view mounts.
        { host, port, username, password, cols: 120, rows: 32 },
        {
          onOutput: (bytes) => {
            controller.term.write(bytes);
          },
          onStatus: (status) => {
            controller.detachSession();
            controller.notice(status.reason);
            set((s) =>
              s.session
                ? { session: { ...s.session, state: "closed", closedReason: status.reason } }
                : s,
            );
          },
        },
      );
      controller.attachSession(id);
      set({
        session: { id, host, port, username, state: "connected", closedReason: null, controller },
        activeTab: "session",
      });
    } catch (err) {
      controller.dispose();
      throw err;
    } finally {
      set({ connecting: false });
    }
  },

  disconnect: async () => {
    const s = get().session;
    if (s && s.state === "connected") {
      await sshDisconnect(s.id).catch(() => undefined);
    }
  },

  closeTab: async () => {
    const s = get().session;
    if (!s) return;
    if (s.state === "connected") {
      await sshDisconnect(s.id).catch(() => undefined);
    }
    s.controller.dispose();
    set({ session: null, activeTab: "home" });
  },
}));
