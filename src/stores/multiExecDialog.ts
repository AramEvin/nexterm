/** Open/close state of the multi-exec picker. */
import { create } from "zustand";

interface MultiExecDialogState {
  /** Tab whose panes are preselected; `null` = closed. */
  openFor: string | null;
  open: (tabId: string) => void;
  close: () => void;
}

export const useMultiExecDialog = create<MultiExecDialogState>()((set) => ({
  openFor: null,
  open: (tabId) => {
    set({ openFor: tabId });
  },
  close: () => {
    set({ openFor: null });
  },
}));
