/** Dialogs used by the file manager (name input, permissions, name clashes). */
import { create } from "zustand";

import type { Conflict } from "@/types/files";

export interface NameRequest {
  title: string;
  label: string;
  initial: string;
  confirmLabel: string;
  /** Throw an `IpcError` to keep the dialog open and show its message. */
  onSubmit: (name: string) => Promise<void>;
}

export interface ChmodRequest {
  /** Shown in the title ("report.txt" / "3 items"). */
  what: string;
  initialMode: number;
  onSubmit: (mode: number) => Promise<void>;
}

export interface ConflictRequest {
  names: string[];
  targetLabel: string;
  onChoose: (choice: Conflict | null) => void;
}

interface FileDialogState {
  name: NameRequest | null;
  chmod: ChmodRequest | null;
  conflict: ConflictRequest | null;
  askName: (r: NameRequest) => void;
  askChmod: (r: ChmodRequest) => void;
  askConflict: (r: ConflictRequest) => void;
  close: (which: "name" | "chmod" | "conflict") => void;
}

export const useFileDialogStore = create<FileDialogState>()((set) => ({
  name: null,
  chmod: null,
  conflict: null,
  askName: (name) => {
    set({ name });
  },
  askChmod: (chmod) => {
    set({ chmod });
  },
  askConflict: (conflict) => {
    set({ conflict });
  },
  close: (which) => {
    set({ [which]: null });
  },
}));

/** Promise wrapper around the clash dialog. `null` = cancelled. */
export function chooseConflict(names: string[], targetLabel: string): Promise<Conflict | null> {
  return new Promise((resolve) => {
    useFileDialogStore.getState().askConflict({ names, targetLabel, onChoose: resolve });
  });
}
