/** Open/close state of the "New local terminal" dialog. */
import { create } from "zustand";

interface LocalDialogState {
  open: boolean;
  show: () => void;
  close: () => void;
}

export const useLocalDialog = create<LocalDialogState>()((set) => ({
  open: false,
  show: () => {
    set({ open: true });
  },
  close: () => {
    set({ open: false });
  },
}));
