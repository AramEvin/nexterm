import { useEffect, useState } from "react";

import { copyLocalPathsInto } from "@/lib/fileActions";
import type { PaneIndex } from "@/stores/files";

/** What the pointer is over during a drag: a pane, optionally a folder row in it. */
export interface DropTarget {
  pane: PaneIndex;
  /** Folder row path, or "" for the pane's current folder. */
  dir: string;
}

export function dropTargetAt(x: number, y: number): DropTarget | null {
  const el = document.elementFromPoint(x, y);
  const paneEl = el?.closest<HTMLElement>("[data-pane-index]");
  if (!paneEl) return null;
  const pane = paneEl.dataset.paneIndex === "1" ? 1 : 0;
  const dir = el?.closest<HTMLElement>("[data-dir-path]")?.dataset.dirPath ?? "";
  return { pane, dir };
}

/**
 * Files dragged in from Windows Explorer. Tauri reports the drop position in
 * physical pixels; divide by the scale factor to find the element under it.
 */
export function useExplorerDrop(tabId: string, visible: boolean) {
  const [target, setTarget] = useState<DropTarget | null>(null);

  useEffect(() => {
    if (!visible) return;
    const state: { unlisten?: () => void; disposed: boolean } = { disposed: false };
    void (async () => {
      try {
        const { getCurrentWebview } = await import("@tauri-apps/api/webview");
        const stop = await getCurrentWebview().onDragDropEvent((event) => {
          const p = event.payload;
          const at = (pos: { x: number; y: number }) =>
            dropTargetAt(pos.x / window.devicePixelRatio, pos.y / window.devicePixelRatio);
          if (p.type === "enter" || p.type === "over") {
            setTarget(at(p.position));
          } else if (p.type === "drop") {
            const t = at(p.position);
            setTarget(null);
            if (t && p.paths.length > 0) {
              void copyLocalPathsInto(tabId, t.pane, p.paths, t.dir || undefined);
            }
          } else {
            setTarget(null);
          }
        });
        if (state.disposed) stop();
        else state.unlisten = stop;
      } catch {
        // Not running inside Tauri (tests / browser preview): no native drop.
      }
    })();
    return () => {
      state.disposed = true;
      state.unlisten?.();
    };
  }, [tabId, visible]);

  return target;
}
