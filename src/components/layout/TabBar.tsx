import { useEffect, useRef, useState } from "react";
import {
  Copy,
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
import { cn } from "@/lib/utils";
import { HOME_TAB, tabTitle, useSessionStore, type SessionTab } from "@/stores/session";

interface TabBarProps {
  onNewSession: () => void;
}

/** MIME type for tab drag-and-drop, so dropping text/files from outside is ignored. */
const TAB_DRAG_TYPE = "application/x-nexterm-tab";

/** Tab strip: Home + one tab per session. Scrolls horizontally when full. */
export function TabBar({ onNewSession }: TabBarProps) {
  const tabs = useSessionStore((s) => s.tabs);
  const activeTabId = useSessionStore((s) => s.activeTabId);
  const setActiveTab = useSessionStore((s) => s.setActiveTab);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Keep the active tab in view (e.g. after opening the 25th tab).
  useEffect(() => {
    scrollRef.current
      ?.querySelector<HTMLElement>(`[data-tab-id="${CSS.escape(activeTabId)}"]`)
      ?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [activeTabId, tabs.length]);

  return (
    <div className="bg-sidebar border-sidebar-border flex h-10 shrink-0 items-end border-b pl-2">
      <div
        ref={scrollRef}
        role="tablist"
        aria-label="Open sessions"
        className="flex min-w-0 [scrollbar-width:thin] items-end gap-1 overflow-x-auto"
        onWheel={(e) => {
          // Vertical mouse wheel scrolls the tab strip horizontally.
          if (scrollRef.current && Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
            scrollRef.current.scrollLeft += e.deltaY;
          }
        }}
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

        {tabs.map((tab) => (
          <SessionTabItem key={tab.id} tab={tab} active={activeTabId === tab.id} />
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
    </div>
  );
}

function tabClass(active: boolean): string {
  return cn(
    "group flex h-8 shrink-0 cursor-default items-center gap-2 rounded-t-md border border-b-0 px-3 text-sm select-none",
    active ? "bg-background" : "text-muted-foreground hover:text-foreground border-transparent",
  );
}

function SessionTabItem({ tab, active }: { tab: SessionTab; active: boolean }) {
  const setActiveTab = useSessionStore((s) => s.setActiveTab);
  const closeTab = useSessionStore((s) => s.closeTab);
  const closeOtherTabs = useSessionStore((s) => s.closeOtherTabs);
  const duplicate = useSessionStore((s) => s.duplicate);
  const reconnect = useSessionStore((s) => s.reconnect);
  const disconnect = useSessionStore((s) => s.disconnect);
  const renameTab = useSessionStore((s) => s.renameTab);
  const moveTab = useSessionStore((s) => s.moveTab);
  const tabCount = useSessionStore((s) => s.tabs.length);

  const [editing, setEditing] = useState(false);
  const [dropTarget, setDropTarget] = useState(false);
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
          draggable={!editing}
          onClick={() => {
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
          onDragStart={(e) => {
            e.dataTransfer.setData(TAB_DRAG_TYPE, tab.id);
            e.dataTransfer.effectAllowed = "move";
          }}
          onDragOver={(e) => {
            if (e.dataTransfer.types.includes(TAB_DRAG_TYPE)) {
              e.preventDefault();
              setDropTarget(true);
            }
          }}
          onDragLeave={() => {
            setDropTarget(false);
          }}
          onDrop={(e) => {
            setDropTarget(false);
            const draggedId = e.dataTransfer.getData(TAB_DRAG_TYPE);
            if (draggedId) {
              e.preventDefault();
              moveTab(draggedId, tab.id);
            }
          }}
          className={cn(tabClass(active), "max-w-60", dropTarget && "ring-brand ring-2 ring-inset")}
        >
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
          onSelect={() => void duplicate(tab.id)}
        >
          <Copy /> Duplicate session
        </ContextMenuItem>
        {canReconnect ? (
          <ContextMenuItem onSelect={() => void reconnect(tab.id)}>
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
