/**
 * Which modal dialog is open. Any component can open one; `App` renders them.
 */
import { create } from "zustand";

import type { SavedSession } from "@/types/library";

export type ConnectTarget = { mode: "quick" } | { mode: "saved"; session: SavedSession };

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
