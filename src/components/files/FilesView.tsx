import { useEffect, useRef, useState } from "react";
import { Upload } from "lucide-react";

import { FilePane } from "@/components/files/FilePane";
import { TransferQueue } from "@/components/files/TransferQueue";
import { dropTargetAt, useExplorerDrop, type DropTarget } from "@/components/files/useFileDrop";
import { describeItems, copyToOtherPane } from "@/lib/fileActions";
import { useFilesStore, type FilesTab, type PaneIndex } from "@/stores/files";

/** Pixels the pointer must move before a press on a row becomes a drag. */
const DRAG_THRESHOLD_PX = 6;

/**
 * Drag rows from one pane to the other with pointer events (HTML5 drag and
 * drop doesn't work in WebView2 while native file drop is on).
 */
function useRowDrag(tabId: string) {
  const drag = useRef<{
    from: PaneIndex;
    paths: string[];
    x: number;
    y: number;
    active: boolean;
  } | null>(null);
  const [target, setTarget] = useState<DropTarget | null>(null);
  const [ghost, setGhost] = useState<{ x: number; y: number; label: string } | null>(null);

  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      const d = drag.current;
      if (!d) return;
      if (!d.active) {
        if (Math.hypot(e.clientX - d.x, e.clientY - d.y) < DRAG_THRESHOLD_PX) return;
        d.active = true;
      }
      document.getSelection()?.removeAllRanges();
      const t = dropTargetAt(e.clientX, e.clientY);
      // Only the other pane accepts a drop.
      setTarget(t && t.pane !== d.from ? t : null);
      setGhost({ x: e.clientX, y: e.clientY, label: describeItems(d.paths) });
    };
    const end = (drop: boolean) => (e: PointerEvent | FocusEvent) => {
      const d = drag.current;
      drag.current = null;
      setGhost(null);
      setTarget(null);
      if (!d?.active || !drop || !("clientX" in e)) return;
      const t = dropTargetAt(e.clientX, e.clientY);
      if (t && t.pane !== d.from) {
        void copyToOtherPane(tabId, d.from, d.paths, t.dir || undefined);
      }
    };
    const onUp = end(true);
    const onCancel = end(false);
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("blur", onCancel);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("blur", onCancel);
    };
  }, [tabId]);

  const start = (from: PaneIndex, paths: string[], e: React.PointerEvent) => {
    if (paths.length === 0) return;
    drag.current = { from, paths, x: e.clientX, y: e.clientY, active: false };
  };

  return { start, target, ghost };
}

/** A file-manager tab: two panes side by side, transfer queue underneath. */
export function FilesView({ tab, visible }: { tab: FilesTab; visible: boolean }) {
  const { start, target: rowTarget, ghost } = useRowDrag(tab.id);
  const explorerTarget = useExplorerDrop(tab.id, visible);
  const target = rowTarget ?? explorerTarget;
  const setActivePane = useFilesStore((s) => s.setActivePane);

  // Ctrl+Tab-free pane switching: Alt+Left / Alt+Right.
  useEffect(() => {
    if (!visible) return;
    const onKey = (e: KeyboardEvent) => {
      if (!e.altKey) return;
      if (e.key === "ArrowLeft") setActivePane(tab.id, 0);
      if (e.key === "ArrowRight") setActivePane(tab.id, 1);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [visible, tab.id, setActivePane]);

  return (
    <div className={visible ? "flex h-full flex-col" : "hidden"}>
      <div className="flex min-h-0 flex-1">
        {([0, 1] as const).map((i) => (
          <div key={i} className="flex min-w-0 flex-1">
            {i === 1 && <div className="bg-border w-px shrink-0" aria-hidden />}
            <FilePane
              tabId={tab.id}
              index={i}
              pane={tab.panes[i]}
              active={tab.activePane === i}
              dropDir={target?.pane === i ? target.dir : null}
              onDragStart={start}
            />
          </div>
        ))}
      </div>
      <TransferQueue />
      {ghost && (
        <div
          className="bg-popover pointer-events-none fixed z-50 flex items-center gap-2 rounded-md border px-2 py-1 text-xs shadow-md"
          style={{ left: ghost.x + 14, top: ghost.y + 10 }}
        >
          <Upload className="text-brand size-3.5" />
          {rowTarget ? `Copy ${ghost.label}` : ghost.label}
        </div>
      )}
    </div>
  );
}
