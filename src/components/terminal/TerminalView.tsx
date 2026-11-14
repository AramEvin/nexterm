import "@xterm/xterm/css/xterm.css";

import { useEffect, useRef } from "react";
import { Loader2, RotateCw, Unplug, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useSessionStore, type SessionTab } from "@/stores/session";

interface TerminalViewProps {
  tab: SessionTab;
  visible: boolean;
}

/** Hosts one tab's xterm instance. Stays mounted while hidden so state survives. */
export function TerminalView({ tab, visible }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const closeTab = useSessionStore((s) => s.closeTab);
  const reconnect = useSessionStore((s) => s.reconnect);
  const { controller } = tab;

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
            <Button size="sm" onClick={() => void reconnect(tab.id)}>
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
      <div className="min-h-0 flex-1 bg-[#11141a] p-1.5">
        <div ref={containerRef} className="h-full w-full" data-testid="terminal" />
      </div>
    </div>
  );
}
