/**
 * Vault state and saved credentials (metadata only; secrets never reach JS).
 */
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";

import {
  credentialCreatePassword,
  credentialDelete,
  credentialImportKey,
  credentialList,
  credentialUpdate,
  IpcError,
  vaultChangePassword,
  vaultForgetDevice,
  vaultLock,
  vaultSetAutoLock,
  vaultSetup,
  vaultStatus,
  vaultTouch,
  vaultUnlock,
} from "@/lib/ipc";
import type { Credential, VaultStatus } from "@/types/ipc";

/** Activity pings to the backend are throttled to one per this interval. */
const TOUCH_INTERVAL_MS = 30_000;

interface VaultState {
  status: VaultStatus | null;
  /** Vault unavailable (e.g. database error) or not loaded yet. */
  error: string | null;
  credentials: Credential[];

  load: () => Promise<void>;
  setup: (masterPassword: string, remember: boolean) => Promise<void>;
  unlock: (masterPassword: string, remember: boolean) => Promise<void>;
  lock: () => Promise<void>;
  forgetDevice: () => Promise<void>;
  changePassword: (oldPassword: string, newPassword: string) => Promise<void>;
  setAutoLock: (minutes: number) => Promise<void>;

  loadCredentials: () => Promise<void>;
  createPassword: (name: string, username: string, password: string) => Promise<Credential>;
  importKey: (
    name: string,
    username: string,
    passphrase: string | null,
  ) => Promise<Credential | null>;
  updateCredential: (
    id: string,
    name: string,
    username: string,
    password: string | null,
  ) => Promise<void>;
  deleteCredential: (id: string) => Promise<void>;
}

export const useVaultStore = create<VaultState>()((set, get) => ({
  status: null,
  error: null,
  credentials: [],

  load: async () => {
    try {
      set({ status: await vaultStatus(), error: null });
    } catch (err) {
      set({ error: err instanceof IpcError ? err.message : "The vault is unavailable." });
    }
    await get().loadCredentials();
  },

  setup: async (masterPassword, remember) => {
    set({ status: await vaultSetup(masterPassword, remember) });
  },

  unlock: async (masterPassword, remember) => {
    set({ status: await vaultUnlock(masterPassword, remember) });
  },

  lock: async () => {
    set({ status: await vaultLock() });
  },

  forgetDevice: async () => {
    set({ status: await vaultForgetDevice() });
  },

  changePassword: (oldPassword, newPassword) => vaultChangePassword(oldPassword, newPassword),

  setAutoLock: async (minutes) => {
    set({ status: await vaultSetAutoLock(minutes) });
  },

  loadCredentials: async () => {
    try {
      set({ credentials: await credentialList() });
    } catch {
      set({ credentials: [] });
    }
  },

  createPassword: async (name, username, password) => {
    const cred = await credentialCreatePassword(name, username, password);
    await get().loadCredentials();
    return cred;
  },

  importKey: async (name, username, passphrase) => {
    const cred = await credentialImportKey(name, username, passphrase);
    await get().loadCredentials();
    return cred;
  },

  updateCredential: async (id, name, username, password) => {
    await credentialUpdate(id, name, username, password);
    await get().loadCredentials();
  },

  deleteCredential: async (id) => {
    await credentialDelete(id);
    await get().loadCredentials();
  },
}));

/**
 * Start-up wiring: keep status in sync with backend auto-lock and report user
 * activity (throttled) so auto-lock only triggers when the user is away.
 */
export function initVaultWatchers(): () => void {
  let lastTouch = 0;
  const onActivity = () => {
    const now = Date.now();
    if (now - lastTouch < TOUCH_INTERVAL_MS) return;
    if (!useVaultStore.getState().status?.unlocked) return;
    lastTouch = now;
    void vaultTouch().catch(() => undefined);
  };
  window.addEventListener("keydown", onActivity, { capture: true });
  window.addEventListener("pointerdown", onActivity, { capture: true });

  let unlisten: (() => void) | null = null;
  void listen("vault-locked", () => {
    void useVaultStore.getState().load();
  })
    .then((fn) => {
      unlisten = fn;
    })
    .catch(() => undefined); // not running inside Tauri (browser preview)

  return () => {
    window.removeEventListener("keydown", onActivity, { capture: true });
    window.removeEventListener("pointerdown", onActivity, { capture: true });
    unlisten?.();
  };
}
