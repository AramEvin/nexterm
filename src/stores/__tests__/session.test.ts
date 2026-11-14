/**
 * Tab store tests. IPC and xterm are mocked: these cover tab lifecycle logic
 * (ordering, cleanup, stale status handling), not the terminal itself.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { SshSessionCallbacks } from "@/lib/ipc";

// ---- mocks -----------------------------------------------------------------

const controllers: { disposed: boolean; attached: string | null }[] = [];

vi.mock("@/lib/terminal", () => ({
  DEFAULT_COLS: 120,
  DEFAULT_ROWS: 32,
  TerminalController: class {
    disposed = false;
    attached: string | null = null;
    size = { cols: 100, rows: 30 };
    term = { write: vi.fn() };
    constructor() {
      controllers.push(this);
    }
    attachSession(id: string) {
      this.attached = id;
    }
    detachSession() {
      this.attached = null;
    }
    notice = vi.fn();
    dispose() {
      this.disposed = true;
    }
  },
}));

let counter = 0;
const lastCallbacks: SshSessionCallbacks[] = [];
const ipc = vi.hoisted(() => ({
  sshConnect: vi.fn(),
  sshReconnect: vi.fn(),
  sshDuplicate: vi.fn(),
  sshDisconnect: vi.fn(() => Promise.resolve()),
  sshForget: vi.fn(() => Promise.resolve()),
}));

vi.mock("@/lib/ipc", () => ({
  IpcError: class IpcError extends Error {
    constructor(
      public code: string,
      message: string,
    ) {
      super(message);
    }
  },
  ...ipc,
}));

function okSession(_a: unknown, ...rest: unknown[]) {
  const callbacks = rest[rest.length - 1] as SshSessionCallbacks;
  lastCallbacks.push(callbacks);
  counter += 1;
  return Promise.resolve({ sessionId: `s${counter}`, connectionId: `c${counter}` });
}

const { useSessionStore, HOME_TAB, tabTitle } = await import("@/stores/session");

const form = { host: "srv", port: 22, username: "root", password: "pw" };
const store = () => useSessionStore.getState();

beforeEach(() => {
  useSessionStore.setState({ tabs: [], activeTabId: HOME_TAB, connecting: false });
  controllers.length = 0;
  lastCallbacks.length = 0;
  counter = 0;
  vi.clearAllMocks();
  ipc.sshConnect.mockImplementation(okSession);
  ipc.sshReconnect.mockImplementation(okSession);
  ipc.sshDuplicate.mockImplementation(okSession);
});

// ---- tests -----------------------------------------------------------------

describe("tabs", () => {
  it("connect opens a new active tab and never stores the password", async () => {
    await store().connect(form);
    const [tab] = store().tabs;
    expect(tab?.state).toBe("connected");
    expect(tab?.sessionId).toBe("s1");
    expect(tab?.connectionId).toBe("c1");
    expect(store().activeTabId).toBe(tab?.id);
    expect(
      JSON.stringify(store().tabs, (k, v: unknown) => (k === "controller" ? undefined : v)),
    ).not.toContain('"pw"');
  });

  it("connect from a saved session sets title, colour and link", async () => {
    await store().connect(form, { title: "prod-db", color: "#ef4444", savedSessionId: "x" });
    const tab = store().tabs[0]!;
    expect(tabTitle(tab)).toBe("prod-db");
    expect(tab.color).toBe("#ef4444");
    expect(tab.savedSessionId).toBe("x");
    await store().duplicate(tab.id);
    expect(store().tabs[1]!.color).toBe("#ef4444");
  });

  it("failed connect leaves no tab and disposes the terminal", async () => {
    ipc.sshConnect.mockRejectedValueOnce(new Error("auth"));
    await expect(store().connect(form)).rejects.toThrow();
    expect(store().tabs).toHaveLength(0);
    expect(controllers[0]?.disposed).toBe(true);
  });

  it("closing a tab disconnects, forgets credentials and activates a neighbour", async () => {
    await store().connect(form);
    await store().connect(form);
    await store().connect(form);
    const [a, b, c] = store().tabs;
    store().setActiveTab(b!.id);

    await store().closeTab(b!.id);
    expect(ipc.sshDisconnect).toHaveBeenCalledWith("s2");
    expect(ipc.sshForget).toHaveBeenCalledWith("c2");
    expect(store().tabs.map((t) => t.id)).toEqual([a!.id, c!.id]);
    expect(store().activeTabId).toBe(c!.id); // right neighbour
    // Other sessions untouched.
    expect(store().tabs.every((t) => t.state === "connected")).toBe(true);

    await store().closeTab(c!.id);
    expect(store().activeTabId).toBe(a!.id); // left neighbour
    await store().closeTab(a!.id);
    expect(store().activeTabId).toBe(HOME_TAB);
  });

  it("closeOtherTabs keeps only the given tab", async () => {
    for (let i = 0; i < 4; i++) await store().connect(form);
    const keep = store().tabs[1]!;
    await store().closeOtherTabs(keep.id);
    expect(store().tabs.map((t) => t.id)).toEqual([keep.id]);
    expect(ipc.sshForget).toHaveBeenCalledTimes(3);
    expect(store().activeTabId).toBe(keep.id);
  });

  it("100 open/close cycles leave nothing behind", async () => {
    for (let i = 0; i < 100; i++) {
      await store().connect(form);
      await store().closeTab(store().tabs[0]!.id);
    }
    expect(store().tabs).toHaveLength(0);
    expect(controllers).toHaveLength(100);
    expect(controllers.every((c) => c.disposed)).toBe(true);
    expect(ipc.sshForget).toHaveBeenCalledTimes(100);
  });

  it("20 tabs: closing one leaves the other 19 connected", async () => {
    for (let i = 0; i < 20; i++) await store().connect(form);
    await store().closeTab(store().tabs[7]!.id);
    expect(store().tabs).toHaveLength(19);
    expect(store().tabs.every((t) => t.state === "connected")).toBe(true);
    expect(controllers.filter((c) => c.disposed)).toHaveLength(1);
  });
});

describe("session status", () => {
  it("remote close marks the tab closed with the reason", async () => {
    await store().connect(form);
    lastCallbacks[0]!.onStatus({ kind: "closed", reason: "Shell exited." });
    const tab = store().tabs[0]!;
    expect(tab.state).toBe("closed");
    expect(tab.closedReason).toBe("Shell exited.");
    expect(tab.sessionId).toBeNull();
  });

  it("reconnect reuses the tab and terminal; late status from the old session is ignored", async () => {
    await store().connect(form);
    const tabId = store().tabs[0]!.id;
    lastCallbacks[0]!.onStatus({ kind: "closed", reason: "dropped" });

    await store().reconnect(tabId);
    expect(ipc.sshReconnect).toHaveBeenCalledWith("c1", 100, 30, expect.anything());
    const tab = store().tabs[0]!;
    expect(tab.state).toBe("connected");
    expect(tab.sessionId).toBe("s2");
    expect(controllers).toHaveLength(1); // same terminal, scrollback kept

    // A duplicate/late status from the first session must not close the new one.
    lastCallbacks[0]!.onStatus({ kind: "closed", reason: "late" });
    expect(store().tabs[0]!.state).toBe("connected");
  });

  it("failed reconnect shows the error and stays reconnectable", async () => {
    await store().connect(form);
    const tabId = store().tabs[0]!.id;
    lastCallbacks[0]!.onStatus({ kind: "closed", reason: "dropped" });
    const { IpcError } = await import("@/lib/ipc");
    ipc.sshReconnect.mockRejectedValueOnce(new IpcError("CONNECTION", "Host unreachable."));

    await store().reconnect(tabId);
    const tab = store().tabs[0]!;
    expect(tab.state).toBe("closed");
    expect(tab.closedReason).toBe("Host unreachable.");
    expect(tab.connectionId).toBe("c1");
  });
});

describe("duplicate, rename, reorder", () => {
  it("duplicate opens a new tab right after the source with its own connection", async () => {
    await store().connect(form);
    await store().connect(form);
    const [a, b] = store().tabs;
    await store().duplicate(a!.id);
    const tabs = store().tabs;
    expect(tabs.map((t) => t.id)).toEqual([a!.id, tabs[1]!.id, b!.id]);
    expect(tabs[1]!.connectionId).toBe("c3");
    expect(tabs[1]!.state).toBe("connected");
    expect(ipc.sshDuplicate).toHaveBeenCalledWith("c1", 100, 30, expect.anything());
    expect(store().activeTabId).toBe(tabs[1]!.id);
  });

  it("rename trims, caps length and resets on empty", async () => {
    await store().connect(form);
    const id = store().tabs[0]!.id;
    store().renameTab(id, "  prod-db  ");
    expect(tabTitle(store().tabs[0]!)).toBe("prod-db");
    store().renameTab(id, "x".repeat(200));
    expect(store().tabs[0]!.customTitle).toHaveLength(64);
    store().renameTab(id, "   ");
    expect(tabTitle(store().tabs[0]!)).toBe("root@srv");
  });

  it("moveTab reorders", async () => {
    for (let i = 0; i < 3; i++) await store().connect(form);
    const [a, b, c] = store().tabs.map((t) => t.id);
    store().moveTab(c!, a!);
    expect(store().tabs.map((t) => t.id)).toEqual([c, a, b]);
    store().moveTab(c!, b!);
    expect(store().tabs.map((t) => t.id)).toEqual([a, b, c]);
  });
});
