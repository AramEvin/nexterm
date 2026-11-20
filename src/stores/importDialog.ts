/** Open/close state of the session import dialog. */
import { create } from "zustand";

interface ImportDialogState {
  open: boolean;
  show: () => void;
  close: () => void;
}

export const useImportDialog = create<ImportDialogState>()((set) => ({
  open: false,
  show: () => {
    set({ open: true });
  },
  close: () => {
    set({ open: false });
  },
}));
