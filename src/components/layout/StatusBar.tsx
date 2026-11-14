import { Circle, Layers, Lock } from "lucide-react";

import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/session";
import { useUiStore } from "@/stores/ui";

const STATE_LABEL = {
  connecting: "connecting…",
  connected: "connected",
  closed: "disconnected",
} as const;

/** Bottom status bar: active tab's connection on the left, app/vault state on the right. */
export function StatusBar() {
  const appInfo = useUiStore((s) => s.appInfo);
  const tab = useSessionStore((s) => s.tabs.find((t) => t.id === s.activeTabId));
  const tabCount = useSessionStore((s) => s.tabs.length);
  const connectedCount = useSessionStore(
    (s) => s.tabs.filter((t) => t.state === "connected").length,
  );

  return (
    <footer className="bg-sidebar text-muted-foreground border-sidebar-border flex h-6 shrink-0 items-center justify-between border-t px-3 text-xs">
      <div className="flex min-w-0 items-center gap-1.5">
        <Circle
          className={cn(
            "size-2 shrink-0 fill-current",
            !tab && "opacity-50",
            tab?.state === "connected" && "text-emerald-400",
            tab?.state === "connecting" && "text-amber-400",
            tab?.state === "closed" && "text-destructive",
          )}
        />
        <span className="truncate">
          {tab ? (
            <>
              <span className="text-foreground">{tab.username}</span>@{tab.host}:{tab.port} —{" "}
              {STATE_LABEL[tab.state]}
            </>
          ) : (
            "No active session"
          )}
        </span>
      </div>
      <div className="flex shrink-0 items-center gap-3">
        {tabCount > 0 && (
          <span className="flex items-center gap-1" title="Connected / open tabs">
            <Layers className="size-3" /> {connectedCount}/{tabCount}
          </span>
        )}
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
