/** Edit rows follow Rust's events; a second Edit of the same file is ignored. */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { EditEvent } from "@/types/files";

const handlers: ((e: EditEvent) => void)[] = [];
const startMock = vi.hoisted(() => vi.fn());
const stopMock = vi.hoisted(() => vi.fn(() => Promise.resolve(true)));
vi.mock("@/lib/ipc", () => ({
  IpcError: class IpcError extends Error {},
  editStart: startMock,
  editStop: stopMock,
}));

const { useEditStore } = await import("@/stores/edits");

beforeEach(() => {
  handlers.length = 0;
  startMock.mockReset();
  startMock.mockImplementation((_e: unknown, _p: string, onEvent: (e: EditEvent) => void) => {
    handlers.push(onEvent);
    return Promise.resolve("b1");
  });
  useEditStore.setState({ items: [] });
});

describe("edit store", () => {
  it("tracks opened, uploaded and closed", async () => {
    const ep = { kind: "remote" as const, fsId: "fs" };
    await useEditStore.getState().start(ep, "/etc/app.conf", "u@h:22");
    await useEditStore.getState().start(ep, "/etc/app.conf", "u@h:22");
    expect(startMock).toHaveBeenCalledTimes(1);
    const send = handlers[0];
    send?.({ type: "opened", localPath: "C:\\Temp\\app.conf", editor: "notepad.exe" });
    send?.({ type: "uploaded", bytes: 10, atMs: 1000 });
    let item = useEditStore.getState().items[0];
    expect(item?.status).toBe("editing");
    expect(item?.saves).toBe(1);
    expect(item?.name).toBe("app.conf");
    await useEditStore.getState().stop(item?.id ?? "");
    expect(stopMock).toHaveBeenCalledWith("b1");
    send?.({ type: "closed" });
    item = useEditStore.getState().items[0];
    expect(item?.status).toBe("closed");
  });
});
