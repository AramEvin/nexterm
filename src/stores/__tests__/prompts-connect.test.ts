/** Prompt queue and connect-error routing. */
import { beforeEach, describe, expect, it, vi } from "vitest";

const respondMock = vi.hoisted(() => vi.fn(() => Promise.resolve()));
vi.mock("@/lib/ipc", () => ({
  IpcError: class IpcError extends Error {
    constructor(
      public code: string,
      message: string,
    ) {
      super(message);
    }
  },
  sshPromptRespond: respondMock,
}));
vi.mock("@/stores/session", () => ({ useSessionStore: { getState: vi.fn() } }));
vi.mock("@/stores/library", () => ({ useLibraryStore: { getState: vi.fn() } }));

const { usePromptStore } = await import("@/stores/prompts");
const { useDialogStore } = await import("@/stores/dialogs");
const { handleConnectError } = await import("@/lib/connect");

const hostPrompt = (id: string) => ({
  type: "hostKey" as const,
  promptId: id,
  kind: "ssh" as const,
  host: "h",
  port: 22,
  keyType: "ssh-ed25519",
  fingerprint: "SHA256:x",
  otherKnownTypes: [],
});

beforeEach(() => {
  usePromptStore.setState({ queue: [] });
  useDialogStore.setState({ unlock: null, alert: null, hostKeyChanged: null, connect: null });
  respondMock.mockClear();
});

describe("prompt queue", () => {
  it("answers in order and forwards to Rust", async () => {
    usePromptStore.getState().push(hostPrompt("a"));
    usePromptStore.getState().push(hostPrompt("b"));
    expect(usePromptStore.getState().queue[0]?.promptId).toBe("a");
    await usePromptStore.getState().respond("a", { type: "hostKey", decision: "reject" });
    expect(respondMock).toHaveBeenCalledWith("a", { type: "hostKey", decision: "reject" });
    expect(usePromptStore.getState().queue.map((p) => p.promptId)).toEqual(["b"]);
  });
});

describe("handleConnectError", () => {
  const ctx = { host: "srv", port: 22, retry: vi.fn(), needCredentials: vi.fn() };

  it("locked vault opens the unlock screen with a retry", () => {
    handleConnectError("VAULT_LOCKED", "locked", ctx);
    const unlock = useDialogStore.getState().unlock;
    expect(unlock).not.toBeNull();
    unlock?.onUnlocked?.();
    expect(ctx.retry).toHaveBeenCalled();
  });

  it("changed host key opens the blocking warning, never an alert", () => {
    handleConnectError("HOST_KEY_CHANGED", "WARNING ...", ctx);
    expect(useDialogStore.getState().hostKeyChanged).toEqual({
      host: "srv",
      port: 22,
      message: "WARNING ...",
    });
    expect(useDialogStore.getState().alert).toBeNull();
  });

  it("missing credentials ask; user decisions stay silent; others alert", () => {
    handleConnectError("CREDENTIALS_REQUIRED", "Enter the password", ctx);
    expect(ctx.needCredentials).toHaveBeenCalledWith("Enter the password");
    handleConnectError("HOST_KEY_REJECTED", "x", ctx);
    handleConnectError("CANCELLED", "x", ctx);
    expect(useDialogStore.getState().alert).toBeNull();
    handleConnectError("AUTH_FAILED", "Authentication failed.", ctx);
    expect(useDialogStore.getState().alert?.message).toBe("Authentication failed.");
  });
});
