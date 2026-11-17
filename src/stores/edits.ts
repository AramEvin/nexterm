/** Remote files open in the local editor (each save is uploaded back by Rust). */
import { create } from "zustand";

import { baseName } from "@/lib/files";
import { editStart, editStop, IpcError } from "@/lib/ipc";
import { useDialogStore } from "@/stores/dialogs";
import { useFilesStore } from "@/stores/files";
import type { EditEvent, Endpoint } from "@/types/files";

export type EditStatus = "opening" | "editing" | "closed";

export interface EditItem {
  id: string;
  backendId: string | null;
  name: string;
  remotePath: string;
  endpoint: Endpoint;
  /** Pane label ("user@host:port"). */
  where: string;
  status: EditStatus;
  editor: string;
  lastSavedAt: number | null;
  saves: number;
  /** Last upload error (the file stays open; the next save retries). */
  error: string | null;
}

interface EditState {
  items: EditItem[];
  start: (endpoint: Endpoint, path: string, where: string) => Promise<void>;
  stop: (id: string) => Promise<void>;
  remove: (id: string) => void;
}

export const useEditStore = create<EditState>()((set, get) => {
  const patch = (id: string, p: Partial<EditItem>) => {
    set((s) => ({ items: s.items.map((e) => (e.id === id ? { ...e, ...p } : e)) }));
  };

  const onEvent = (id: string) => (event: EditEvent) => {
    switch (event.type) {
      case "opened":
        patch(id, { status: "editing", editor: event.editor });
        break;
      case "uploaded": {
        const item = get().items.find((e) => e.id === id);
        if (item) {
          // Show the new size / time in panes that show this folder.
          const dir = item.remotePath.slice(0, item.remotePath.lastIndexOf("/")) || "/";
          useFilesStore.getState().refreshWhere(item.endpoint, dir);
        }
        patch(id, {
          lastSavedAt: event.atMs,
          saves: (get().items.find((e) => e.id === id)?.saves ?? 0) + 1,
          error: null,
        });
        break;
      }
      case "failed":
        patch(id, { error: event.message });
        break;
      case "closed":
        patch(id, { status: "closed" });
        break;
    }
  };

  return {
    items: [],

    start: async (endpoint, path, where) => {
      // Already open: don't download a second copy.
      if (
        get().items.some((e) => e.remotePath === path && e.status !== "closed" && e.where === where)
      ) {
        return;
      }
      const id = crypto.randomUUID();
      set((s) => ({
        items: [
          ...s.items.filter((e) => e.status !== "closed").slice(-49),
          {
            id,
            backendId: null,
            name: baseName(path),
            remotePath: path,
            endpoint,
            where,
            status: "opening",
            editor: "",
            lastSavedAt: null,
            saves: 0,
            error: null,
          },
        ],
      }));
      try {
        const backendId = await editStart(endpoint, path, onEvent(id));
        patch(id, { backendId });
      } catch (err) {
        get().remove(id);
        useDialogStore.getState().openAlert({
          title: "Can't edit this file",
          message: err instanceof IpcError ? err.message : "Opening the editor failed.",
        });
      }
    },

    stop: async (id) => {
      const item = get().items.find((e) => e.id === id);
      if (item?.backendId) await editStop(item.backendId).catch(() => false);
    },

    remove: (id) => {
      set((s) => ({ items: s.items.filter((e) => e.id !== id) }));
    },
  };
});
