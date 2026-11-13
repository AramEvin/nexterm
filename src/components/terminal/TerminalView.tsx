import "@xterm/xterm/css/xterm.css";

import { useEffect, useRef } from "react";
import { Unplug, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import type { LiveSession } from "@/stores/session";
import { useSessionStore } from "@/stores/session";

interface TerminalViewProps {
  session: LiveSession;
  visible: boolean;
}

/** Hosts the session's xterm instance. Stays mounted while hidden so state survives. */
export function TerminalView({ session, visible }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const closeTab = useSessionStore((s) => s.closeTab);
  const { controller } = session;

  useEffect(() => {
    if (containerRef.current) controller.open(containerRef.current);
  }, [controller]);

  useEffect(() => {
    if (visible) controller.focus();
  }, [visible, controller]);

  return (
    <div className={visible ? "relative flex h-full flex-col" : "hidden"}>
      {session.state === "closed" && (
        <div
          role="status"
          className="bg-destructive/15 border-destructive/30 text-foreground flex items-center gap-3 border-b px-4 py-2 text-sm"
        >
          <Unplug className="text-destructive size-4 shrink-0" />
          <span className="flex-1">
            <strong className="font-medium">Disconnected.</strong> {session.closedReason}
          </span>
          <Button size="sm" variant="outline" onClick={() => void closeTab()}>
            <X /> Close tab
          </Button>
        </div>
      )}
      <div className="min-h-0 flex-1 bg-[#11141a] p-1.5">
        <div ref={containerRef} className="h-full w-full" data-testid="terminal" />
      </div>
    </div>
  );
}
