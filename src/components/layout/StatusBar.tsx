import { Circle, Lock } from "lucide-react";

import { useUiStore } from "@/stores/ui";

/** Bottom status bar: connection state on the left, app/vault state on the right. */
export function StatusBar() {
  const appInfo = useUiStore((s) => s.appInfo);

  return (
    <footer className="bg-sidebar text-muted-foreground border-sidebar-border flex h-6 shrink-0 items-center justify-between border-t px-3 text-xs">
      <div className="flex items-center gap-1.5">
        <Circle className="size-2 fill-current opacity-50" />
        <span>No active session</span>
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
