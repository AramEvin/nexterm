/** Transfer queue: events from Rust drive each row; retry re-sends the request. */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { TransferEvent, TransferRequest } from "@/types/files";

const handlers: ((e: TransferEvent) => void)[] = [];
const startMock = vi.hoisted(() => vi.fn());
const cancelMock = vi.hoisted(() => vi.fn(() => Promise.resolve(true)));
vi.mock("@/lib/ipc", () => ({
  IpcError: class IpcError extends Error {
    constructor(
      public code: string,
      message: string,
    ) {
      super(message);
    }
  },
  transferStart: startMock,
  transferCancel: cancelMock,
}));

const { useTransferStore } = await import("@/stores/transfers");

const request: TransferRequest = {
  source: { kind: "local" },
  sources: ["/tmp/a"],
  target: { kind: "remote", fsId: "fs1" },
  targetDir: "/home/u",
  conflict: "overwrite",
};

beforeEach(() => {
  handlers.length = 0;
  startMock.mockReset();
  startMock.mockImplementation((_req: TransferRequest, onEvent: (e: TransferEvent) => void) => {
    handlers.push(onEvent);
    return Promise.resolve(`backend-${String(handlers.length)}`);
  });
  useTransferStore.setState({ items: [] });
});

describe("transfer queue", () => {
  it("tracks progress and calls onFinished once", async () => {
    const finished = vi.fn();
    await useTransferStore
      .getState()
      .start(request, { title: "a", from: "Local", to: "srv", onFinished: finished });
    const send = handlers[0];
    expect(send).toBeDefined();
    send?.({ type: "started", filesTotal: 1, bytesTotal: 100 });
    send?.({
      type: "progress",
      filesDone: 0,
      filesTotal: 1,
      bytesDone: 40,
      bytesTotal: 100,
      bytesPerSec: 10,
      current: "a",
    });
    let item = useTransferStore.getState().items[0];
    expect(item?.status).toBe("running");
    expect(item?.bytesDone).toBe(40);
    expect(item?.backendId).toBe("backend-1");
    send?.({ type: "done", filesDone: 1, bytesDone: 100, skipped: 0, skippedNames: [] });
    item = useTransferStore.getState().items[0];
    expect(item?.status).toBe("done");
    expect(finished).toHaveBeenCalledTimes(1);
  });

  it("cancels through Rust and retries a failed copy with the same request", async () => {
    await useTransferStore.getState().start(request, { title: "a", from: "x", to: "y" });
    const id = useTransferStore.getState().items[0]?.id ?? "";
    await useTransferStore.getState().cancel(id);
    expect(cancelMock).toHaveBeenCalledWith("backend-1");
    handlers[0]?.({ type: "failed", message: "Permission denied", filesDone: 0, bytesDone: 0 });
    expect(useTransferStore.getState().items[0]?.message).toBe("Permission denied");

    await useTransferStore.getState().retry(id);
    expect(startMock).toHaveBeenCalledTimes(2);
    expect(startMock.mock.calls[1]?.[0]).toEqual(request);
    expect(useTransferStore.getState().items).toHaveLength(1);
    expect(useTransferStore.getState().items[0]?.status).toBe("queued");
  });

  it("marks a request Rust refused as failed", async () => {
    const { IpcError } = await import("@/lib/ipc");
    startMock.mockImplementationOnce(() =>
      Promise.reject(new IpcError("VALIDATION", "Nothing selected.")),
    );
    await useTransferStore.getState().start(request, { title: "a", from: "x", to: "y" });
    const item = useTransferStore.getState().items[0];
    expect(item?.status).toBe("failed");
    expect(item?.message).toBe("Nothing selected.");
  });
});
