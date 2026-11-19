/**
 * Tab store tests. IPC and xterm are mocked: these cover tab lifecycle logic
 * (ordering, cleanup, stale status handling), not the terminal itself.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { SshSessionCallbacks } from "@/lib/ipc";

// ---- mocks -----------------------------------------------------------------

interface MockController {
  disposed: boolean;
  attached: string | null;
  sendKey: ReturnType<typeof vi.fn>;
  pasteReviewed: ReturnType<typeof vi.fn>;
  onUserInput: ((input: unknown) => void) | null;
  pasteGuard: ((text: string) => Promise<string | null>) | null;
}
const controllers: MockController[] = [];

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
    sendKey = vi.fn();
    pasteReviewed = vi.fn();
    onUserInput: ((input: unknown) => void) | null = null;
    pasteGuard: ((text: string) => Promise<string | null>) | null = null;
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

const { useSessionStore, HOME_TAB, tabTitle, panesOf, activePane } =
  await import("@/stores/session");
const { usePasteStore } = await import("@/stores/paste");

const form = { host: "srv", port: 22, username: "root", password: "pw" };
const store = () => useSessionStore.getState();

beforeEach(() => {
  useSessionStore.setState({
    tabs: [],
    activeTabId: HOME_TAB,
    connecting: false,
    layouts: {},
    focused: {},
    multiExec: [],
    sidePanels: [],
  });
  usePasteStore.setState({ current: null });
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

describe("split panes", () => {
  it("splitting adds a pane inside the tab and focuses it", async () => {
    await store().connect(form);
    const root = store().tabs[0]!;
    await store().splitPane(root.id, "row");
    const [, pane] = store().tabs;
    expect(pane?.parentId).toBe(root.id);
    expect(pane?.state).toBe("connected");
    expect(ipc.sshDuplicate).toHaveBeenCalledWith("c1", 100, 30, expect.anything());
    expect(panesOf(store(), root.id).map((t) => t.id)).toEqual([root.id, pane?.id]);
    expect(store().focused[root.id]).toBe(pane?.id);
    expect(activePane(store())?.id).toBe(pane?.id);
    expect(store().activeTabId).toBe(root.id);
  });

  it("closing a split pane keeps the tab; the last split removes the layout", async () => {
    await store().connect(form);
    const root = store().tabs[0]!;
    await store().splitPane(root.id, "row");
    const pane = store().tabs[1]!;
    await store().closePane(pane.id);
    expect(store().tabs.map((t) => t.id)).toEqual([root.id]);
    expect(store().layouts[root.id]).toBeUndefined();
    expect(ipc.sshForget).toHaveBeenCalledWith("c2");
    expect(controllers[1]?.disposed).toBe(true);
  });

  it("closing the tab's own pane hands the tab to the next pane", async () => {
    await store().connect(form, { title: "prod" });
    await store().connect(form); // a second tab after it
    const [root, other] = store().tabs;
    store().setActiveTab(root!.id);
    await store().splitPane(root!.id, "row");
    await store().splitPane(root!.id, "column");
    const heirs = store().tabs.filter((t) => t.parentId === root!.id);
    expect(heirs).toHaveLength(2);
    await store().closePane(root!.id);
    const tabsNow = store().tabs.filter((t) => t.parentId === null);
    const heir = tabsNow[0]!;
    expect(tabsNow.map((t) => t.id)).toEqual([heir.id, other!.id]); // same position
    expect(heir.customTitle).toBe("prod");
    expect(store().activeTabId).toBe(heir.id);
    expect(panesOf(store(), heir.id)).toHaveLength(2);
    expect(store().tabs.find((t) => t.id !== heir.id && t.id !== other!.id)?.parentId).toBe(
      heir.id,
    );
  });

  it("closing a tab closes all its panes", async () => {
    await store().connect(form);
    const root = store().tabs[0]!;
    await store().splitPane(root.id, "row");
    await store().splitPane(root.id, "column");
    await store().closeTab(root.id);
    expect(store().tabs).toHaveLength(0);
    expect(store().layouts).toEqual({});
    expect(ipc.sshForget).toHaveBeenCalledTimes(3);
    expect(controllers.every((c) => c.disposed)).toBe(true);
    expect(store().activeTabId).toBe(HOME_TAB);
  });

  it("stops at the pane limit", async () => {
    await store().connect(form);
    const root = store().tabs[0]!;
    for (let i = 0; i < 12; i++) await store().splitPane(root.id, "row");
    expect(panesOf(store(), root.id).length).toBe(8);
  });
});

describe("multi-exec", () => {
  const ctl = (i: number) => store().tabs[i]!.controller as unknown as MockController;

  it("needs two terminals and copies typed keys to the others only", async () => {
    await store().connect(form);
    await store().connect(form);
    await store().connect(form);
    const [a, b] = store().tabs;
    store().setMultiExec([a!.id]);
    expect(store().multiExec).toEqual([]);
    store().setMultiExec([a!.id, b!.id, "unknown"]);
    expect(store().multiExec).toEqual([a!.id, b!.id]);
    ctl(0).onUserInput?.({ kind: "key", data: "l" });
    expect(ctl(1).sendKey).toHaveBeenCalledWith("l");
    expect(ctl(0).sendKey).not.toHaveBeenCalled();
    expect(ctl(2).sendKey).not.toHaveBeenCalled();
    ctl(1).onUserInput?.({ kind: "paste", text: "uptime" });
    expect(ctl(0).pasteReviewed).toHaveBeenCalledWith("uptime");
    // Typing in a terminal outside multi-exec goes nowhere else.
    ctl(2).onUserInput?.({ kind: "key", data: "x" });
    expect(ctl(0).sendKey).not.toHaveBeenCalled();
  });

  it("closing a member removes it; pastes in multi-exec always ask", async () => {
    await store().connect(form);
    await store().connect(form);
    const [a, b] = store().tabs;
    store().setMultiExec([a!.id, b!.id]);
    const pending = ctl(0).pasteGuard?.("ls");
    expect(usePasteStore.getState().current?.targets).toBe(2);
    usePasteStore.getState().answer(null);
    await expect(pending).resolves.toBeNull();
    await store().closeTab(b!.id);
    expect(store().multiExec).toEqual([]); // one left: multi-exec ends
    // A single-line paste outside multi-exec needs no dialog.
    await expect(ctl(0).pasteGuard?.("ls")).resolves.toBe("ls");
  });
});
