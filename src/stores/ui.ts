/**
 * UI layout state (zustand).
 *
 * zustand in one sentence: `create()` returns a React hook; components call
 * `useUiStore((s) => s.field)` and re-render only when that field changes.
 */
import { create } from "zustand";

import type { AppInfo } from "@/types/ipc";
import { getAppInfo } from "@/lib/ipc";

interface UiState {
  sidebarCollapsed: boolean;
  appInfo: AppInfo | null;
  toggleSidebar: () => void;
  loadAppInfo: () => Promise<void>;
}

export const useUiStore = create<UiState>()((set) => ({
  sidebarCollapsed: false,
  appInfo: null,
  toggleSidebar: () => {
    set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed }));
  },
  loadAppInfo: async () => {
    try {
      set({ appInfo: await getAppInfo() });
    } catch {
      // Running in a plain browser (`npm run dev`) has no Tauri backend; ignore.
      set({ appInfo: null });
    }
  },
}));
