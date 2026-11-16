import { Circle, Layers, Lock, LockOpen, ShieldOff } from "lucide-react";

import { cn } from "@/lib/utils";
import { useDialogStore } from "@/stores/dialogs";
import { useSessionStore } from "@/stores/session";
import { useVaultStore } from "@/stores/vault";
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
        <VaultIndicator />
        <span title={appInfo?.logDir}>
          {appInfo ? `${appInfo.name} v${appInfo.version}` : "NexTerm (browser preview)"}
        </span>
      </div>
    </footer>
  );
}

/** Click: set up / unlock / open the manager. Shows the vault state at a glance. */
function VaultIndicator() {
  const status = useVaultStore((s) => s.status);
  const lock = useVaultStore((s) => s.lock);
  const openVaultSetup = useDialogStore((s) => s.openVaultSetup);
  const openUnlock = useDialogStore((s) => s.openUnlock);
  const openCredentials = useDialogStore((s) => s.openCredentials);
  if (!status) return null;

  if (!status.initialized) {
    return (
      <button
        type="button"
        className="hover:text-foreground flex items-center gap-1"
        onClick={openVaultSetup}
      >
        <ShieldOff className="size-3" /> Vault: not set up
      </button>
    );
  }
  if (!status.unlocked) {
    return (
      <button
        type="button"
        className="hover:text-foreground flex items-center gap-1"
        onClick={() => {
          openUnlock();
        }}
      >
        <Lock className="size-3" /> Vault: locked
      </button>
    );
  }
  return (
    <span className="flex items-center gap-1">
      <button
        type="button"
        className="hover:text-foreground flex items-center gap-1 text-emerald-400"
        onClick={openCredentials}
      >
        <LockOpen className="size-3" /> Vault: unlocked
      </button>
      <button
        type="button"
        className="hover:text-foreground ml-1 rounded px-1"
        aria-label="Lock the vault"
        title="Lock the vault"
        onClick={() => void lock()}
      >
        <Lock className="size-3" />
      </button>
    </span>
  );
}
