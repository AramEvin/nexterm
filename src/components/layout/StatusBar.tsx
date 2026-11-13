import { Circle, Lock } from "lucide-react";

import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/session";
import { useUiStore } from "@/stores/ui";

/** Bottom status bar: connection state on the left, app/vault state on the right. */
export function StatusBar() {
  const appInfo = useUiStore((s) => s.appInfo);
  const session = useSessionStore((s) => s.session);

  const connected = session?.state === "connected";
  const label = session
    ? `${session.username}@${session.host}:${session.port} — ${connected ? "connected" : "disconnected"}`
    : "No active session";

  return (
    <footer className="bg-sidebar text-muted-foreground border-sidebar-border flex h-6 shrink-0 items-center justify-between border-t px-3 text-xs">
      <div className="flex items-center gap-1.5">
        <Circle
          className={cn(
            "size-2 fill-current",
            !session && "opacity-50",
            connected && "text-emerald-400",
            session && !connected && "text-destructive",
          )}
        />
        <span>{label}</span>
      </div>
      <div className="flex items-center gap-3">
        <span className="flex items-center gap-1">
          <Lock className="size-3" /> Vault: not configured
        </span>
        <span title={appInfo?.logDir}>
          {appInfo ? `${appInfo.name} v${appInfo.version}` : "NexTerm (browser preview)"}
        </span>
      </div>
    </footer>
  );
}
