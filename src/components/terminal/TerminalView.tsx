import "@xterm/xterm/css/xterm.css";

import { useEffect, useRef } from "react";
import { Columns2, FolderTree, Loader2, Radio, RotateCw, Rows2, Unplug, X } from "lucide-react";

import { SidePanel } from "@/components/files/SidePanel";
import { ResourceBar } from "@/components/terminal/ResourceBar";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { reconnectTab, splitTab } from "@/lib/connect";
import { MAX_PANES } from "@/lib/splits";
import { cn } from "@/lib/utils";
import { useFilesStore } from "@/stores/files";
import { tabTitle, useSessionStore, type SessionTab } from "@/stores/session";

interface TerminalViewProps {
  tab: SessionTab;
  /** Its tab is the active one. */
  visible: boolean;
  /** Part of a split tab (shows a title strip and a close-pane button). */
  split: boolean;
  /** Keyboard focus belongs here within its tab. */
  focused: boolean;
  /** Number of panes in its tab. */
  paneCount: number;
}

/** One terminal (a tab or a split pane). Stays mounted while hidden so state survives. */
export function TerminalView({ tab, visible, split, focused, paneCount }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const closeTab = useSessionStore((s) => s.closeTab);
  const closePane = useSessionStore((s) => s.closePane);
  const focusPane = useSessionStore((s) => s.focusPane);
  const sideOpen = useSessionStore((s) => s.sidePanels.includes(tab.id));
  const multiExec = useSessionStore((s) => s.multiExec.includes(tab.id));
  const toggleSide = useSessionStore((s) => s.toggleSidePanel);
  const closeSide = useFilesStore((s) => s.closeSide);
  const { controller } = tab;
  // A split tab shows one bar for the whole tab (TerminalArea), not one per pane.
  const showBar = !split && tab.resourceMonitor && tab.state === "connected";

  // Closing the terminal closes its SFTP panel connection too.
  useEffect(() => {
    const id = tab.id;
    return () => {
      closeSide(id);
    };
  }, [tab.id, closeSide]);

  useEffect(() => {
    if (containerRef.current) controller.open(containerRef.current);
  }, [controller]);

  useEffect(() => {
    controller.setVisible(visible);
  }, [visible, controller]);

  // Size changes from the side panel, the resource bar or the split layout.
  useEffect(() => {
    controller.refit();
  }, [sideOpen, showBar, split, controller]);

  useEffect(() => {
    if (visible && focused) controller.focus();
  }, [visible, focused, controller]);

  const closeLabel = split ? "Close pane" : "Close tab";
  const close = () => void (split ? closePane(tab.id) : closeTab(tab.id));

  return (
    <div
      data-pane-id={tab.id}
      className={cn(
        "relative flex h-full flex-col overflow-hidden",
        split && "rounded-sm border",
        split && (focused ? "border-brand/70" : "border-border"),
        multiExec && "border-2 border-orange-500",
      )}
      onPointerDownCapture={() => {
        focusPane(tab.id);
      }}
      onFocusCapture={() => {
        focusPane(tab.id);
      }}
    >
      {(split || multiExec) && (
        <div
          className={cn(
            "flex h-6 shrink-0 items-center gap-1.5 border-b px-2 text-xs",
            focused ? "bg-accent text-foreground" : "bg-sidebar text-muted-foreground",
          )}
        >
          {multiExec && (
            <span className="flex items-center gap-1 font-medium text-orange-400">
              <Radio className="size-3" /> MULTI-EXEC
            </span>
          )}
          {tab.color && (
            <span
              aria-hidden
              className="size-2 shrink-0 rounded-full"
              style={{ backgroundColor: tab.color }}
            />
          )}
          <span className="truncate" title={`${tab.username}@${tab.host}:${String(tab.port)}`}>
            {tabTitle(tab)}
          </span>
          {split && (
            <button
              type="button"
              aria-label="Close pane"
              title="Close pane (Ctrl+Shift+W)"
              className="hover:bg-background -mr-1 ml-auto rounded p-0.5"
              onClick={close}
            >
              <X className="size-3" />
            </button>
          )}
        </div>
      )}
      {tab.state === "closed" && (
        <div
          role="status"
          className="bg-destructive/15 border-destructive/30 text-foreground flex items-center gap-3 border-b px-4 py-2 text-sm"
        >
          <Unplug className="text-destructive size-4 shrink-0" />
          <span className="min-w-0 flex-1 truncate">
            <strong className="font-medium">Disconnected.</strong> {tab.closedReason}
          </span>
          {tab.connectionId !== null && (
            <Button size="sm" onClick={() => void reconnectTab(tab.id)}>
              <RotateCw /> Reconnect
            </Button>
          )}
          <Button size="sm" variant="outline" onClick={close}>
            <X /> {closeLabel}
          </Button>
        </div>
      )}
      {tab.state === "connecting" && (
        <div
          role="status"
          className="bg-muted/50 text-muted-foreground flex items-center gap-2 border-b px-4 py-2 text-sm"
        >
          <Loader2 className="size-4 animate-spin" /> Connecting to {tab.host}:{tab.port}…
        </div>
      )}
      <div className="flex min-h-0 flex-1">
        <div className="group relative min-w-0 flex-1 bg-[#11141a] p-1.5">
          <div ref={containerRef} className="h-full w-full" data-testid="terminal" />
          {tab.connectionId !== null && (
            <div className="absolute top-2 right-4 z-10 flex gap-1 opacity-40 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
              <PaneButton
                label="Split right (Alt+Shift+=)"
                disabled={paneCount >= MAX_PANES}
                onClick={() => void splitTab(tab.id, "row")}
              >
                <Columns2 />
              </PaneButton>
              <PaneButton
                label="Split down (Alt+Shift+-)"
                disabled={paneCount >= MAX_PANES}
                onClick={() => void splitTab(tab.id, "column")}
              >
                <Rows2 />
              </PaneButton>
              <PaneButton
                label={sideOpen ? "Hide SFTP panel" : "Show SFTP panel"}
                pressed={sideOpen}
                onClick={() => {
                  toggleSide(tab.id);
                }}
              >
                <FolderTree />
              </PaneButton>
            </div>
          )}
        </div>
        {sideOpen && (
          <SidePanel
            terminal={tab}
            visible={visible}
            onClose={() => {
              toggleSide(tab.id);
            }}
          />
        )}
      </div>
      {showBar && <ResourceBar tab={tab} visible={visible} />}
    </div>
  );
}

function PaneButton({
  label,
  pressed,
  disabled,
  onClick,
  children,
}: {
  label: string;
  pressed?: boolean;
  disabled?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="secondary"
          size="icon-sm"
          aria-label={label}
          aria-pressed={pressed}
          disabled={disabled}
          onClick={onClick}
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
