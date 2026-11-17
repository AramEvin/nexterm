/**
 * Connection flows that may need the user: unlock the vault, type a missing
 * password, or react to a changed host key. Every error is routed here so
 * the reaction is the same wherever a connection is started.
 */
import { IpcError } from "@/lib/ipc";
import { useDialogStore } from "@/stores/dialogs";
import { useLibraryStore } from "@/stores/library";
import { openFilesForSaved } from "@/stores/files";
import { useSessionStore } from "@/stores/session";
import type { AppErrorCode } from "@/types/ipc";
import { isFileProtocol, type SavedSession } from "@/types/library";

interface ErrorContext {
  host: string;
  port: number;
  /** Retry after the vault was unlocked. */
  retry: () => void;
  /** Ask for a missing username/password (saved sessions). */
  needCredentials?: (reason: string) => void;
}

/** React to a connect error. Returns true if a dialog took over. */
export function handleConnectError(code: AppErrorCode, message: string, ctx: ErrorContext): void {
  const dialogs = useDialogStore.getState();
  switch (code) {
    case "VAULT_LOCKED":
      dialogs.openUnlock(ctx.retry);
      return;
    case "CREDENTIALS_REQUIRED":
      if (ctx.needCredentials) ctx.needCredentials(message);
      else dialogs.openAlert({ title: "Connection needs input", message });
      return;
    case "HOST_KEY_CHANGED":
      dialogs.openHostKeyChanged({ host: ctx.host, port: ctx.port, message });
      return;
    case "HOST_KEY_REJECTED":
    case "CANCELLED":
      return; // the user's own decision; nothing to report
    default:
      dialogs.openAlert({ title: "Connection failed", message });
  }
}

/** Double-click / Enter / "Connect" on a saved session. */
export async function openSavedSession(
  session: SavedSession,
  extra?: { username?: string; password?: string },
): Promise<void> {
  // FTP/FTPS sessions have no terminal: open them in the file manager.
  if (isFileProtocol(session.protocol)) {
    openFilesForSaved(session);
    return;
  }
  try {
    await useSessionStore.getState().connectSaved(session, extra);
    void useLibraryStore.getState().load(); // refresh "last connected"
  } catch (err) {
    const { code, message } =
      err instanceof IpcError ? err : { code: "INTERNAL" as const, message: "Connection failed." };
    handleConnectError(code, message, {
      host: session.host,
      port: session.port,
      retry: () => void openSavedSession(session, extra),
      needCredentials: (reason) => {
        useDialogStore.getState().openConnect({ mode: "saved", session, reason });
      },
    });
  }
}

/** Reconnect button / menu item. */
export async function reconnectTab(tabId: string): Promise<void> {
  const code = await useSessionStore.getState().reconnect(tabId);
  const tab = useSessionStore.getState().tabs.find((t) => t.id === tabId);
  if (!code || !tab) return;
  if (code === "VAULT_LOCKED" || code === "HOST_KEY_CHANGED") {
    handleConnectError(code, tab.closedReason ?? "", {
      host: tab.host,
      port: tab.port,
      retry: () => void reconnectTab(tabId),
    });
  }
}

/** Duplicate tab (vault may be needed for saved sessions). */
export async function duplicateTab(tabId: string): Promise<void> {
  const before = new Set(useSessionStore.getState().tabs.map((t) => t.id));
  const code = await useSessionStore.getState().duplicate(tabId);
  if (code !== "VAULT_LOCKED" && code !== "HOST_KEY_CHANGED") return;
  const state = useSessionStore.getState();
  const fresh = state.tabs.find((t) => !before.has(t.id));
  if (!fresh) return;
  handleConnectError(code, fresh.closedReason ?? "", {
    host: fresh.host,
    port: fresh.port,
    // Retry in the tab that already exists.
    retry: () => void reconnectTabFromDuplicate(fresh.id, tabId),
  });
}

/** A failed duplicate has no connection id yet: close it and duplicate again. */
async function reconnectTabFromDuplicate(failedTabId: string, sourceTabId: string): Promise<void> {
  await useSessionStore.getState().closeTab(failedTabId);
  await duplicateTab(sourceTabId);
}
