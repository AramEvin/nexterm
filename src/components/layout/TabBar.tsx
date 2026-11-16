import { useEffect, useRef, useState } from "react";
import {
  Copy,
  FolderOpen,
  FolderSync,
  Home,
  Loader2,
  Pencil,
  Plus,
  RotateCw,
  TerminalSquare,
  Unplug,
  X,
  XSquare,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { duplicateTab, reconnectTab } from "@/lib/connect";
import { cn } from "@/lib/utils";
import { openFilesForTerminal, useFilesStore, type FilesTab } from "@/stores/files";
import { HOME_TAB, tabTitle, useSessionStore, type SessionTab } from "@/stores/session";

interface TabBarProps {
  onNewSession: () => void;
}

/** Pixels the pointer must travel before a press becomes a drag. */
const DRAG_THRESHOLD_PX = 5;

interface DragState {
  tabId: string;
  startX: number;
  active: boolean;
  overId: string | null;
}

/**
 * Tab reordering with pointer events instead of HTML5 drag-and-drop: on
 * Windows, Tauri's native file-drop handling (needed for SFTP uploads from
 * Explorer in Phase 5) swallows HTML5 drag events inside the webview.
 */
function useTabDrag(moveTab: (tabId: string, targetId: string) => void) {
  const drag = useRef<DragState | null>(null);
  const suppressClick = useRef(false);
  const [overId, setOverId] = useState<string | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);

  const tabIdAt = (x: number, y: number): string | null => {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-tab-id]");
    const id = el?.dataset.tabId;
    return id && id !== HOME_TAB ? id : null;
  };

  const onPointerDown = (tabId: string) => (e: React.PointerEvent<HTMLElement>) => {
    if (e.button !== 0) return;
    drag.current = { tabId, startX: e.clientX, active: false, overId: null };
  };

  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      const d = drag.current;
      if (!d) return;
      if (!d.active) {
        if (Math.abs(e.clientX - d.startX) < DRAG_THRESHOLD_PX) return;
        d.active = true;
        setDraggingId(d.tabId);
      }
      document.getSelection()?.removeAllRanges();
      const over = tabIdAt(e.clientX, e.clientY);
      const target = over !== d.tabId ? over : null;
      if (target !== d.overId) {
        d.overId = target;
        setOverId(target);
      }
    };
    const onUp = () => {
      const d = drag.current;
      drag.current = null;
      if (!d?.active) return;
      suppressClick.current = true; // the click that follows a drag is not a "select"
      if (d.overId) moveTab(d.tabId, d.overId);
      setOverId(null);
      setDraggingId(null);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("blur", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("blur", onUp);
    };
  }, [moveTab]);

  /** Returns true (and resets) if the current click ends a drag. */
  const consumeDragClick = () => {
    const was = suppressClick.current;
    suppressClick.current = false;
    return was;
  };

  return { onPointerDown, overId, draggingId, consumeDragClick };
}

/** Tab strip: pinned Home tab + one tab per session (scrolls horizontally when full). */
export function TabBar({ onNewSession }: TabBarProps) {
  const tabs = useSessionStore((s) => s.tabs);
  const filesTabs = useFilesStore((s) => s.tabs);
  const openFiles = useFilesStore((s) => s.openTab);
  const activeTabId = useSessionStore((s) => s.activeTabId);
  const setActiveTab = useSessionStore((s) => s.setActiveTab);
  const moveTab = useSessionStore((s) => s.moveTab);
  const scrollRef = useRef<HTMLDivElement>(null);
  const { onPointerDown, overId, draggingId, consumeDragClick } = useTabDrag(moveTab);

  // Keep the active tab in view (e.g. after opening the 25th tab).
  useEffect(() => {
    scrollRef.current
      ?.querySelector<HTMLElement>(`[data-tab-id="${CSS.escape(activeTabId)}"]`)
      ?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [activeTabId, tabs.length, filesTabs.length]);

  return (
    <div
      role="tablist"
      aria-label="Open sessions"
      className="bg-sidebar border-sidebar-border flex h-10 shrink-0 items-end border-b pl-2"
    >
      <div
        role="tab"
        data-tab-id={HOME_TAB}
        aria-selected={activeTabId === HOME_TAB}
        tabIndex={0}
        onClick={() => {
          setActiveTab(HOME_TAB);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") setActiveTab(HOME_TAB);
        }}
        className={tabClass(activeTabId === HOME_TAB)}
      >
        <Home className="text-brand size-3.5" />
        Home
      </div>

      <div
        ref={scrollRef}
        className="ml-1 flex min-w-0 [scrollbar-width:none] items-end gap-1 overflow-x-auto [&::-webkit-scrollbar]:hidden"
        onWheel={(e) => {
          // Vertical mouse wheel scrolls the tab strip horizontally.
          if (scrollRef.current && Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
            scrollRef.current.scrollLeft += e.deltaY;
          }
        }}
      >
        {tabs.map((tab) => (
          <SessionTabItem
            key={tab.id}
            tab={tab}
            active={activeTabId === tab.id}
            dropTarget={overId === tab.id}
            dragging={draggingId === tab.id}
            onPointerDown={onPointerDown(tab.id)}
            consumeDragClick={consumeDragClick}
          />
        ))}
        {filesTabs.map((tab, i) => (
          <FilesTabItem key={tab.id} tab={tab} number={i + 1} active={activeTabId === tab.id} />
        ))}
      </div>

      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="icon-sm"
            className="mx-1 mb-0.5 shrink-0"
            aria-label="New session"
            onClick={onNewSession}
          >
            <Plus />
          </Button>
        </TooltipTrigger>
        <TooltipContent>Quick connect</TooltipContent>
      </Tooltip>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="icon-sm"
            className="mr-1 mb-0.5 shrink-0"
            aria-label="New file manager"
            onClick={() => {
              openFiles();
            }}
          >
            <FolderOpen />
          </Button>
        </TooltipTrigger>
        <TooltipContent>File manager (SFTP)</TooltipContent>
      </Tooltip>
    </div>
  );
}

/** A file-manager tab (two panes). Title: the remote side, if any. */
function FilesTabItem({ tab, number, active }: { tab: FilesTab; number: number; active: boolean }) {
  const setActiveTab = useSessionStore((s) => s.setActiveTab);
  const closeTab = useFilesStore((s) => s.closeTab);
  const remote = tab.panes.find((p) => p.source && p.source.kind !== "local");
  const title = remote ? `Files: ${remote.label}` : `Files ${String(number)}`;
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          role="tab"
          data-tab-id={tab.id}
          aria-selected={active}
          tabIndex={0}
          title={title}
          onClick={() => {
            setActiveTab(tab.id);
          }}
          onAuxClick={(e) => {
            if (e.button === 1) void closeTab(tab.id);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") setActiveTab(tab.id);
          }}
          className={cn(tabClass(active), "relative max-w-60")}
        >
          <FolderOpen className="text-brand size-3.5 shrink-0" />
          <span className="truncate">{title}</span>
          <button
            type="button"
            aria-label={`Close ${title}`}
            className={cn(
              "hover:bg-accent -mr-1 ml-1 shrink-0 rounded p-0.5",
              !active && "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
            )}
            onClick={(e) => {
              e.stopPropagation();
              void closeTab(tab.id);
            }}
          >
            <X className="size-3.5" />
          </button>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onSelect={() => void closeTab(tab.id)}>
          <X /> Close tab
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

function tabClass(active: boolean): string {
  return cn(
    "group flex h-8 shrink-0 cursor-default items-center gap-2 rounded-t-md border border-b-0 px-3 text-sm select-none",
    active ? "bg-background" : "text-muted-foreground hover:text-foreground border-transparent",
  );
}

interface SessionTabItemProps {
  tab: SessionTab;
  active: boolean;
  dropTarget: boolean;
  dragging: boolean;
  onPointerDown: (e: React.PointerEvent<HTMLElement>) => void;
  consumeDragClick: () => boolean;
}

function SessionTabItem({
  tab,
  active,
  dropTarget,
  dragging,
  onPointerDown,
  consumeDragClick,
}: SessionTabItemProps) {
  const setActiveTab = useSessionStore((s) => s.setActiveTab);
  const closeTab = useSessionStore((s) => s.closeTab);
  const closeOtherTabs = useSessionStore((s) => s.closeOtherTabs);
  const disconnect = useSessionStore((s) => s.disconnect);
  const renameTab = useSessionStore((s) => s.renameTab);
  const tabCount = useSessionStore((s) => s.tabs.length);

  const [editing, setEditing] = useState(false);
  const title = tabTitle(tab);
  const canReconnect = tab.state === "closed" && tab.connectionId !== null;

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          role="tab"
          data-tab-id={tab.id}
          aria-selected={active}
          tabIndex={0}
          title={`${tab.username}@${tab.host}:${tab.port}`}
          onPointerDown={editing ? undefined : onPointerDown}
          onClick={() => {
            if (consumeDragClick()) return;
            setActiveTab(tab.id);
          }}
          onDoubleClick={() => {
            setEditing(true);
          }}
          onAuxClick={(e) => {
            if (e.button === 1) void closeTab(tab.id); // middle-click closes
          }}
          onKeyDown={(e) => {
            if (editing) return;
            if (e.key === "Enter") setActiveTab(tab.id);
            if (e.key === "F2") setEditing(true);
          }}
          className={cn(
            tabClass(active),
            "relative max-w-60",
            dropTarget && "ring-brand ring-2 ring-inset",
            dragging && "opacity-60",
          )}
        >
          {tab.color && (
            <span
              aria-hidden
              className="absolute inset-x-2 top-0 h-0.5 rounded-b"
              style={{ backgroundColor: tab.color }}
            />
          )}
          <StateIcon state={tab.state} />
          {editing ? (
            <RenameInput
              initial={title}
              onDone={(value) => {
                setEditing(false);
                if (value !== null) renameTab(tab.id, value);
              }}
            />
          ) : (
            <span className="truncate">{title}</span>
          )}
          <button
            type="button"
            aria-label={`Close ${title}`}
            className={cn(
              "hover:bg-accent -mr-1 ml-1 shrink-0 rounded p-0.5",
              !active && "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
            )}
            onClick={(e) => {
              e.stopPropagation();
              void closeTab(tab.id);
            }}
          >
            <X className="size-3.5" />
          </button>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem
          onSelect={() => {
            setEditing(true);
          }}
        >
          <Pencil /> Rename
        </ContextMenuItem>
        <ContextMenuItem
          disabled={tab.connectionId === null}
          onSelect={() => void duplicateTab(tab.id)}
        >
          <Copy /> Duplicate session
        </ContextMenuItem>
        <ContextMenuItem
          disabled={tab.connectionId === null}
          onSelect={() => {
            openFilesForTerminal(tab);
          }}
        >
          <FolderSync /> Open files (SFTP)
        </ContextMenuItem>
        {canReconnect ? (
          <ContextMenuItem onSelect={() => void reconnectTab(tab.id)}>
            <RotateCw /> Reconnect
          </ContextMenuItem>
        ) : (
          <ContextMenuItem
            disabled={tab.state !== "connected"}
            onSelect={() => void disconnect(tab.id)}
          >
            <Unplug /> Disconnect
          </ContextMenuItem>
        )}
        <ContextMenuSeparator />
        <ContextMenuItem onSelect={() => void closeTab(tab.id)}>
          <X /> Close tab
        </ContextMenuItem>
        <ContextMenuItem
          disabled={tabCount < 2}
          variant="destructive"
          onSelect={() => void closeOtherTabs(tab.id)}
        >
          <XSquare /> Close other tabs
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

function StateIcon({ state }: { state: SessionTab["state"] }) {
  if (state === "connecting") {
    return <Loader2 className="text-muted-foreground size-3.5 shrink-0 animate-spin" />;
  }
  return (
    <TerminalSquare
      aria-label={state}
      className={cn(
        "size-3.5 shrink-0",
        state === "connected" ? "text-emerald-400" : "text-destructive",
      )}
    />
  );
}

/** Inline title editor. Enter saves, Escape cancels, empty resets to user@host. */
function RenameInput({
  initial,
  onDone,
}: {
  initial: string;
  onDone: (value: string | null) => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  const done = useRef(false);

  useEffect(() => {
    ref.current?.select();
  }, []);

  const finish = (value: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(value);
  };

  return (
    <input
      ref={ref}
      defaultValue={initial}
      maxLength={64}
      aria-label="Tab name"
      className="bg-background w-36 rounded border px-1 text-sm outline-none"
      onClick={(e) => {
        e.stopPropagation();
      }}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") finish(e.currentTarget.value);
        if (e.key === "Escape") finish(null);
      }}
      onBlur={(e) => {
        finish(e.currentTarget.value);
      }}
    />
  );
}
