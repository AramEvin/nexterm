/**
 * Jump hosts (ProxyJump). Mirrors the Rust checks so the editor only offers
 * valid choices; Rust validates again on save and on connect.
 */
import type { SavedSession } from "@/types/library";

/** Longest chain of jump hosts (`MAX_JUMP_HOPS` in Rust). */
export const MAX_JUMP_HOPS = 5;

/** Jump hosts of a session, the one dialled first first. Stops at loops. */
export function jumpChain(sessions: SavedSession[], jumpHostId: string | null): SavedSession[] {
  const byId = new Map(sessions.map((s) => [s.id, s]));
  const out: SavedSession[] = [];
  const seen = new Set<string>();
  let next = jumpHostId;
  while (next !== null && !seen.has(next) && out.length <= MAX_JUMP_HOPS) {
    seen.add(next);
    const hop = byId.get(next);
    if (!hop) break;
    out.unshift(hop);
    next = hop.jumpHostId;
  }
  return out;
}

/**
 * Sessions `selfId` may use as its jump host: SSH, not itself, not one whose
 * own chain passes through `selfId`, and not making the chain too long.
 */
export function jumpCandidates(sessions: SavedSession[], selfId: string | null): SavedSession[] {
  return sessions
    .filter((s) => s.protocol === "ssh" && s.id !== selfId)
    .filter((s) => {
      const chain = jumpChain(sessions, s.id);
      return chain.length <= MAX_JUMP_HOPS && !chain.some((h) => h.id === selfId);
    })
    .sort((a, b) => a.name.localeCompare(b.name));
}

export function hopLabel(s: SavedSession): string {
  return s.username ? `${s.username}@${s.host}` : s.host;
}
