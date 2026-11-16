/**
 * Which modal dialog is open. Any component can open one; `App` renders them.
 */
import { create } from "zustand";

import type { SavedSession } from "@/types/library";

export type ConnectTarget =
  | { mode: "quick" }
  /** Saved session needing a username and/or password (`reason` from Rust). */
  | { mode: "saved"; session: SavedSession; reason?: string };

export interface AlertRequest {
  title: string;
  message: string;
}

/** A changed host key blocked a connection. */
export interface HostKeyChangedInfo {
  host: string;
  port: number;
  message: string;
}

export type EditorTarget =
  { mode: "create"; folderId: string | null } | { mode: "edit"; session: SavedSession };

export interface ConfirmRequest {
  title: string;
  description: string;
  confirmLabel: string;
  destructive?: boolean;
  onConfirm: () => Promise<void> | void;
}

interface DialogState {
  connect: ConnectTarget | null;
  editor: EditorTarget | null;
  confirm: ConfirmRequest | null;
  alert: AlertRequest | null;
  /** Vault setup wizard. */
  vaultSetup: boolean;
  /** Unlock screen; `onUnlocked` retries what needed the vault. */
  unlock: { onUnlocked?: () => void } | null;
  /** Credentials & vault settings manager. */
  credentials: boolean;
  hostKeyChanged: HostKeyChangedInfo | null;
  /** Choose a folder's default credential. */
  folderCredential: { folderId: string } | null;
  openAlert: (alert: AlertRequest) => void;
  closeAlert: () => void;
  openVaultSetup: () => void;
  closeVaultSetup: () => void;
  openUnlock: (onUnlocked?: () => void) => void;
  closeUnlock: () => void;
  openCredentials: () => void;
  closeCredentials: () => void;
  openHostKeyChanged: (info: HostKeyChangedInfo) => void;
  closeHostKeyChanged: () => void;
  openFolderCredential: (folderId: string) => void;
  closeFolderCredential: () => void;
  openConnect: (target: ConnectTarget) => void;
  closeConnect: () => void;
  openEditor: (target: EditorTarget) => void;
  closeEditor: () => void;
  askConfirm: (request: ConfirmRequest) => void;
  closeConfirm: () => void;
}

export const useDialogStore = create<DialogState>()((set) => ({
  connect: null,
  editor: null,
  confirm: null,
  alert: null,
  vaultSetup: false,
  unlock: null,
  credentials: false,
  hostKeyChanged: null,
  folderCredential: null,
  openAlert: (alert) => {
    set({ alert });
  },
  closeAlert: () => {
    set({ alert: null });
  },
  openVaultSetup: () => {
    set({ vaultSetup: true });
  },
  closeVaultSetup: () => {
    set({ vaultSetup: false });
  },
  openUnlock: (onUnlocked) => {
    set({ unlock: { onUnlocked } });
  },
  closeUnlock: () => {
    set({ unlock: null });
  },
  openCredentials: () => {
    set({ credentials: true });
  },
  closeCredentials: () => {
    set({ credentials: false });
  },
  openHostKeyChanged: (hostKeyChanged) => {
    set({ hostKeyChanged });
  },
  closeHostKeyChanged: () => {
    set({ hostKeyChanged: null });
  },
  openFolderCredential: (folderId) => {
    set({ folderCredential: { folderId } });
  },
  closeFolderCredential: () => {
    set({ folderCredential: null });
  },
  openConnect: (connect) => {
    set({ connect });
  },
  closeConnect: () => {
    set({ connect: null });
  },
  openEditor: (editor) => {
    set({ editor });
  },
  closeEditor: () => {
    set({ editor: null });
  },
  askConfirm: (confirm) => {
    set({ confirm });
  },
  closeConfirm: () => {
    set({ confirm: null });
  },
}));
