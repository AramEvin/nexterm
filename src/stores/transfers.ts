/**
 * The transfer queue shown at the bottom of every file-manager tab.
 * Rust runs the copies (max 3 at once) and streams events; this store keeps
 * one row per request so it can show progress, cancel and retry.
 */
import { create } from "zustand";

import { IpcError, transferCancel, transferStart } from "@/lib/ipc";
import type { TransferEvent, TransferRequest } from "@/types/files";

export type TransferStatus = "queued" | "running" | "done" | "failed" | "cancelled";

export interface TransferItem {
  id: string;
  /** Id in Rust; null until `transfer_start` returns. */
  backendId: string | null;
  title: string;
  from: string;
  to: string;
  request: TransferRequest;
  status: TransferStatus;
  filesDone: number;
  filesTotal: number;
  bytesDone: number;
  bytesTotal: number;
  bytesPerSec: number;
  current: string;
  message: string | null;
  skipped: number;
  skippedNames: string[];
}

export interface TransferMeta {
  title: string;
  from: string;
  to: string;
  /** Called once when the copy finishes (any outcome), e.g. to refresh a pane. */
  onFinished?: () => void;
}

/** Finished rows kept before the oldest are dropped. */
const MAX_FINISHED = 200;

interface TransferState {
  items: TransferItem[];
  start: (request: TransferRequest, meta: TransferMeta) => Promise<void>;
  cancel: (id: string) => Promise<void>;
  retry: (id: string) => Promise<void>;
  remove: (id: string) => void;
  clearFinished: () => void;
}

export function isFinished(status: TransferStatus): boolean {
  return status === "done" || status === "failed" || status === "cancelled";
}

const finishers = new Map<string, () => void>();
const metas = new Map<string, TransferMeta>();

export const useTransferStore = create<TransferState>()((set, get) => {
  const patch = (id: string, p: Partial<TransferItem>) => {
    set((s) => ({ items: s.items.map((t) => (t.id === id ? { ...t, ...p } : t)) }));
  };

  const finish = (id: string) => {
    const done = finishers.get(id);
    finishers.delete(id);
    done?.();
  };

  const onEvent = (id: string) => (event: TransferEvent) => {
    switch (event.type) {
      case "queued":
        patch(id, { status: "queued" });
        break;
      case "started":
        patch(id, {
          status: "running",
          filesTotal: event.filesTotal,
          bytesTotal: event.bytesTotal,
        });
        break;
      case "progress":
        patch(id, {
          status: "running",
          filesDone: event.filesDone,
          filesTotal: event.filesTotal,
          bytesDone: event.bytesDone,
          bytesTotal: event.bytesTotal,
          bytesPerSec: event.bytesPerSec,
          current: event.current,
        });
        break;
      case "done":
        patch(id, {
          status: "done",
          filesDone: event.filesDone,
          bytesDone: event.bytesDone,
          skipped: event.skipped,
          skippedNames: event.skippedNames,
          bytesPerSec: 0,
          current: "",
        });
        finish(id);
        break;
      case "failed":
        patch(id, {
          status: "failed",
          message: event.message,
          filesDone: event.filesDone,
          bytesDone: event.bytesDone,
          bytesPerSec: 0,
        });
        finish(id);
        break;
      case "cancelled":
        patch(id, { status: "cancelled", bytesPerSec: 0 });
        finish(id);
        break;
    }
  };

  const launch = async (id: string, request: TransferRequest) => {
    try {
      const backendId = await transferStart(request, onEvent(id));
      patch(id, { backendId });
    } catch (err) {
      patch(id, {
        status: "failed",
        message: err instanceof IpcError ? err.message : "The transfer could not start.",
      });
      finish(id);
    }
  };

  return {
    items: [],

    start: async (request, meta) => {
      const id = crypto.randomUUID();
      metas.set(id, meta);
      if (meta.onFinished) finishers.set(id, meta.onFinished);
      const item: TransferItem = {
        id,
        backendId: null,
        title: meta.title,
        from: meta.from,
        to: meta.to,
        request,
        status: "queued",
        filesDone: 0,
        filesTotal: 0,
        bytesDone: 0,
        bytesTotal: 0,
        bytesPerSec: 0,
        current: "",
        message: null,
        skipped: 0,
        skippedNames: [],
      };
      set((s) => {
        let items = [...s.items, item];
        const finished = items.filter((t) => isFinished(t.status));
        if (finished.length > MAX_FINISHED) {
          const drop = new Set(finished.slice(0, finished.length - MAX_FINISHED).map((t) => t.id));
          items = items.filter((t) => !drop.has(t.id));
        }
        return { items };
      });
      await launch(id, request);
    },

    cancel: async (id) => {
      const item = get().items.find((t) => t.id === id);
      if (!item || isFinished(item.status)) return;
      if (item.backendId) await transferCancel(item.backendId).catch(() => false);
    },

    retry: async (id) => {
      const item = get().items.find((t) => t.id === id);
      if (!item || !isFinished(item.status)) return;
      const meta = metas.get(id);
      get().remove(id);
      await get().start(item.request, {
        title: item.title,
        from: item.from,
        to: item.to,
        onFinished: meta?.onFinished,
      });
    },

    remove: (id) => {
      metas.delete(id);
      finishers.delete(id);
      set((s) => ({ items: s.items.filter((t) => t.id !== id) }));
    },

    clearFinished: () => {
      for (const t of get().items) if (isFinished(t.status)) metas.delete(t.id);
      set((s) => ({ items: s.items.filter((t) => !isFinished(t.status)) }));
    },
  };
});
