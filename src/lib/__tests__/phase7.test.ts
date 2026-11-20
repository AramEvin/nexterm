import { describe, expect, it } from "vitest";

import { MAX_JUMP_HOPS, hopLabel, jumpCandidates, jumpChain } from "@/lib/jump";
import type { SavedSession } from "@/types/library";

const s = (id: string, extra: Partial<SavedSession> = {}): SavedSession => ({
  id,
  folderId: null,
  name: id,
  protocol: "ssh",
  host: `${id}.example.com`,
  port: 22,
  username: "ops",
  authMethod: "password",
  credentialId: null,
  jumpHostId: null,
  color: null,
  notes: "",
  favorite: false,
  resourceMonitor: true,
  tags: [],
  sortOrder: 0,
  createdAt: 0,
  updatedAt: 0,
  lastConnectedAt: null,
  ...extra,
});

describe("jump hosts", () => {
  const lib = [
    s("bastion"),
    s("inner", { jumpHostId: "bastion" }),
    s("db", { jumpHostId: "inner" }),
    s("ftp", { protocol: "ftps" }),
  ];

  it("lists the chain in dialling order", () => {
    expect(jumpChain(lib, "inner").map((x) => x.id)).toEqual(["bastion", "inner"]);
    expect(jumpChain(lib, null)).toEqual([]);
    expect(jumpChain(lib, "missing")).toEqual([]);
  });

  it("stops at loops instead of hanging", () => {
    const loop = [s("a", { jumpHostId: "b" }), s("b", { jumpHostId: "a" })];
    expect(jumpChain(loop, "a").length).toBeLessThanOrEqual(2);
  });

  it("offers only SSH sessions that don't lead back to the edited one", () => {
    const forBastion = jumpCandidates(lib, "bastion").map((x) => x.id);
    // inner and db both go through bastion: picking them would loop.
    expect(forBastion).toEqual([]);
    const forNew = jumpCandidates(lib, null).map((x) => x.id);
    expect(forNew).toEqual(["bastion", "db", "inner"]);
    expect(forNew).not.toContain("ftp");
  });

  it("refuses chains that would get too long", () => {
    const chain: SavedSession[] = [s("h0")];
    for (let i = 1; i <= MAX_JUMP_HOPS; i++)
      chain.push(s(`h${String(i)}`, { jumpHostId: `h${String(i - 1)}` }));
    const ids = jumpCandidates(chain, null).map((x) => x.id);
    expect(ids).toContain(`h${String(MAX_JUMP_HOPS - 1)}`);
    expect(ids).not.toContain(`h${String(MAX_JUMP_HOPS)}`);
  });

  it("labels hops", () => {
    expect(hopLabel(s("x"))).toBe("ops@x.example.com");
    expect(hopLabel(s("y", { username: "" }))).toBe("y.example.com");
  });
});
