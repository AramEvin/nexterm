import { useEffect, useRef } from "react";
import { Radio, Square } from "lucide-react";

import { ResourceBar } from "@/components/terminal/ResourceBar";
import { TerminalView } from "@/components/terminal/TerminalView";
import { Button } from "@/components/ui/button";
import { openDefaultLocalTerminal, splitTab } from "@/lib/connect";
import { layoutRects, leaves, neighbour, type DividerRect, type Rect } from "@/lib/splits";
import { cn } from "@/lib/utils";
import { useMultiExecDialog } from "@/stores/multiExecDialog";
import { activePane, panesOf, rootId, useSessionStore } from "@/stores/session";

const FULL: Rect = { x: 0, y: 0, w: 1, h: 1 };
/** Half the gap between panes, in px. */
const GAP = 2;

/**
 * All terminals, each rendered exactly once and absolutely positioned by its
 * tab's split layout. Changing the layout only moves boxes; xterm instances
 * are never remounted.
 */
export function TerminalArea() {
  const tabs = useSessionStore((s) => s.tabs);
  const layouts = useSessionStore((s) => s.layouts);
  const focused = useSessionStore((s) => s.focused);
  const activeTabId = useSessionStore((s) => s.activeTabId);
  const multiExec = useSessionStore((s) => s.multiExec);
  const areaRef = useRef<HTMLDivElement>(null);
  useSplitShortcuts();

  const activeLayout = layouts[activeTabId];
  // Panes of a split tab are the same server: one resource bar for the tab,
  // read from its first connected pane (stable, so focus changes don't
  // restart the readings).
  const barSource = (() => {
    if (!activeLayout) return undefined;
    const pick = panesOf({ tabs, layouts }, activeTabId).find((p) => p.state === "connected");
    return pick?.resourceMonitor ? pick : undefined;
  })();
  const rectsByTab = new Map<string, Map<string, Rect>>();
  for (const [tabId, layout] of Object.entries(layouts)) {
    rectsByTab.set(tabId, new Map(layoutRects(layout).panes.map((p) => [p.id, p])));
  }

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      {multiExec.length > 0 && <MultiExecBanner count={multiExec.length} />}
      <div ref={areaRef} className="relative min-h-0 flex-1">
        {tabs.map((tab) => {
          const root = rootId(tab);
          const layout = layouts[root];
          const rect = rectsByTab.get(root)?.get(tab.id) ?? FULL;
          const visible = activeTabId === root;
          const split = layout !== undefined;
          const paneCount = layout ? leaves(layout).length : 1;
          return (
            <div
              key={tab.id}
              className={visible ? "absolute" : "hidden"}
              style={split ? paneStyle(rect) : { inset: 0 }}
            >
              <TerminalView
                tab={tab}
                visible={visible}
                split={split}
                focused={(focused[root] ?? root) === tab.id}
                paneCount={paneCount}
              />
            </div>
          );
        })}
        {activeLayout &&
          layoutRects(activeLayout).dividers.map((d) => (
            <Divider
              key={d.path.join("-") || "root"}
              tabId={activeTabId}
              divider={d}
              areaRef={areaRef}
            />
          ))}
      </div>
      {barSource && <ResourceBar key={barSource.id} tab={barSource} visible />}
    </div>
  );
}

function paneStyle(r: Rect): React.CSSProperties {
  const pct = (v: number) => `${String(v * 100)}%`;
  const left = r.x > 0 ? GAP : 0;
  const right = r.x + r.w < 1 - 1e-9 ? GAP : 0;
  const top = r.y > 0 ? GAP : 0;
  const bottom = r.y + r.h < 1 - 1e-9 ? GAP : 0;
  return {
    left: `calc(${pct(r.x)} + ${String(left)}px)`,
    top: `calc(${pct(r.y)} + ${String(top)}px)`,
    width: `calc(${pct(r.w)} - ${String(left + right)}px)`,
    height: `calc(${pct(r.h)} - ${String(top + bottom)}px)`,
  };
}

/** Drag handle between two panes (pointer events: HTML5 drag is unreliable in WebView2). */
function Divider({
  tabId,
  divider,
  areaRef,
}: {
  tabId: string;
  divider: DividerRect;
  areaRef: React.RefObject<HTMLDivElement | null>;
}) {
  const setSplitRatio = useSessionStore((s) => s.setSplitRatio);
  const { area, direction, ratio, path } = divider;
  const row = direction === "row";
  const pct = (v: number) => `${String(v * 100)}%`;
  const style: React.CSSProperties = row
    ? {
        left: `calc(${pct(area.x + area.w * ratio)} - 3px)`,
        top: pct(area.y),
        width: 6,
        height: pct(area.h),
      }
    : {
        top: `calc(${pct(area.y + area.h * ratio)} - 3px)`,
        left: pct(area.x),
        height: 6,
        width: pct(area.w),
      };

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    const el = areaRef.current;
    if (!el) return;
    e.preventDefault();
    const box = el.getBoundingClientRect();
    const handle = e.currentTarget;
    handle.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      const r = row
        ? ((ev.clientX - box.left) / box.width - area.x) / area.w
        : ((ev.clientY - box.top) / box.height - area.y) / area.h;
      setSplitRatio(tabId, path, r);
    };
    const up = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      handle.removeEventListener("pointercancel", up);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
    handle.addEventListener("pointercancel", up);
  };

  return (
    <div
      role="separator"
      aria-orientation={row ? "vertical" : "horizontal"}
      aria-valuenow={Math.round(ratio * 100)}
      aria-label="Resize panes"
      tabIndex={0}
      onPointerDown={onPointerDown}
      onKeyDown={(e) => {
        const step = e.shiftKey ? 0.1 : 0.02;
        const dec = row ? "ArrowLeft" : "ArrowUp";
        const inc = row ? "ArrowRight" : "ArrowDown";
        if (e.key === dec || e.key === inc) {
          e.preventDefault();
          setSplitRatio(tabId, path, ratio + (e.key === inc ? step : -step));
        }
      }}
      className={cn(
        "hover:bg-brand/50 focus-visible:bg-brand/60 absolute z-20 rounded-full outline-none",
        row ? "cursor-col-resize" : "cursor-row-resize",
      )}
      style={style}
    />
  );
}

function MultiExecBanner({ count }: { count: number }) {
  const setMultiExec = useSessionStore((s) => s.setMultiExec);
  const activeTabId = useSessionStore((s) => s.activeTabId);
  const openPicker = useMultiExecDialog((s) => s.open);
  return (
    <div
      role="status"
      className="flex h-8 shrink-0 items-center gap-2 border-b border-orange-500/50 bg-orange-500/15 px-3 text-sm"
    >
      <Radio className="size-4 text-orange-400" />
      <span>
        <strong>Multi-exec is on:</strong> what you type goes to {count} terminals.
      </span>
      <Button
        size="sm"
        variant="ghost"
        className="ml-auto h-6"
        onClick={() => {
          openPicker(activeTabId);
        }}
      >
        Change
      </Button>
      <Button
        size="sm"
        variant="outline"
        className="h-6"
        onClick={() => {
          setMultiExec([]);
        }}
      >
        <Square /> Stop
      </Button>
    </div>
  );
}

/**
 * Alt+Shift+= / Alt+Shift+- split, Alt+Arrow moves focus (only in split
 * tabs, so shells keep Alt+Arrow otherwise), Ctrl+Shift+W closes the pane,
 * Ctrl+Shift+M opens multi-exec. Capture phase: runs before xterm.
 */
function useSplitShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Ctrl+Shift+T: new local terminal, from anywhere.
      if (e.ctrlKey && e.shiftKey && !e.altKey && e.code === "KeyT") {
        if (document.querySelector('[role="dialog"], [role="alertdialog"]')) return;
        e.preventDefault();
        e.stopPropagation();
        void openDefaultLocalTerminal();
        return;
      }
      const state = useSessionStore.getState();
      const pane = activePane(state);
      // Only while a terminal tab is active (not Home or a file manager) and
      // no dialog is open.
      if (!pane || rootId(pane) !== state.activeTabId) return;
      if (document.querySelector('[role="dialog"], [role="alertdialog"]')) return;
      const root = state.activeTabId;
      let handled = true;
      if (e.altKey && e.shiftKey && !e.ctrlKey && (e.code === "Equal" || e.code === "NumpadAdd")) {
        void splitTab(pane.id, "row");
      } else if (
        e.altKey &&
        e.shiftKey &&
        !e.ctrlKey &&
        (e.code === "Minus" || e.code === "NumpadSubtract")
      ) {
        void splitTab(pane.id, "column");
      } else if (e.ctrlKey && e.shiftKey && !e.altKey && e.code === "KeyW") {
        void state.closePane(pane.id);
      } else if (e.ctrlKey && e.shiftKey && !e.altKey && e.code === "KeyM") {
        useMultiExecDialog.getState().open(root);
      } else if (e.altKey && !e.shiftKey && !e.ctrlKey && e.key.startsWith("Arrow")) {
        const layout = state.layouts[root];
        if (!layout) {
          handled = false;
        } else {
          const dir = e.key.slice(5).toLowerCase() as "left" | "right" | "up" | "down";
          const next = neighbour(layout, pane.id, dir);
          if (next) state.focusPane(next);
        }
      } else {
        handled = false;
      }
      if (handled) {
        e.preventDefault();
        e.stopPropagation();
      }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => {
      window.removeEventListener("keydown", onKey, { capture: true });
    };
  }, []);
}
