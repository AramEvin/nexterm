import "@xterm/xterm/css/xterm.css";

import { useEffect, useRef } from "react";
import { FolderTree, Loader2, RotateCw, Unplug, X } from "lucide-react";

import { SidePanel } from "@/components/files/SidePanel";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { reconnectTab } from "@/lib/connect";
import { useFilesStore } from "@/stores/files";
import { useSessionStore, type SessionTab } from "@/stores/session";

interface TerminalViewProps {
  tab: SessionTab;
  visible: boolean;
}

/** Hosts one tab's xterm instance. Stays mounted while hidden so state survives. */
export function TerminalView({ tab, visible }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const closeTab = useSessionStore((s) => s.closeTab);
  const sideOpen = useSessionStore((s) => s.sidePanels.includes(tab.id));
  const toggleSide = useSessionStore((s) => s.toggleSidePanel);
  const closeSide = useFilesStore((s) => s.closeSide);
  const { controller } = tab;

  // Closing the tab closes its SFTP panel connection too.
  useEffect(() => {
    const id = tab.id;
    return () => {
      closeSide(id);
    };
  }, [tab.id, closeSide]);

  // The terminal re-fits itself when the panel takes or gives back width.
  useEffect(() => {
    if (visible) controller.setVisible(true);
  }, [sideOpen, visible, controller]);

  useEffect(() => {
    if (containerRef.current) controller.open(containerRef.current);
  }, [controller]);

  useEffect(() => {
    controller.setVisible(visible);
  }, [visible, controller]);

  return (
    <div className={visible ? "relative flex h-full flex-col" : "hidden"}>
      {tab.state === "closed" && (
        <div
          role="status"
          className="bg-destructive/15 border-destructive/30 text-foreground flex items-center gap-3 border-b px-4 py-2 text-sm"
        >
          <Unplug className="text-destructive size-4 shrink-0" />
          <span className="flex-1">
            <strong className="font-medium">Disconnected.</strong> {tab.closedReason}
          </span>
          {tab.connectionId !== null && (
            <Button size="sm" onClick={() => void reconnectTab(tab.id)}>
              <RotateCw /> Reconnect
            </Button>
          )}
          <Button size="sm" variant="outline" onClick={() => void closeTab(tab.id)}>
            <X /> Close tab
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
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="secondary"
                  size="icon-sm"
                  aria-label={sideOpen ? "Hide SFTP panel" : "Show SFTP panel"}
                  aria-pressed={sideOpen}
                  className="absolute top-2 right-4 z-10 opacity-50 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
                  onClick={() => {
                    toggleSide(tab.id);
                  }}
                >
                  <FolderTree />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{sideOpen ? "Hide" : "Show"} SFTP panel</TooltipContent>
            </Tooltip>
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
    </div>
  );
}
