/**
 * Split-pane layout of one tab: a binary tree whose leaves are terminal ids.
 *
 * Pure functions only (unit-tested). The tree is turned into rectangles so
 * every terminal is rendered once, absolutely positioned, and never
 * remounted when the layout changes (xterm can't be re-opened).
 */

export type SplitDirection = "row" | "column";

export type SplitNode =
  | { kind: "leaf"; id: string }
  | {
      kind: "split";
      /** `row` = side by side, `column` = stacked. */
      direction: SplitDirection;
      /** Share of the first child, 0..1. */
      ratio: number;
      first: SplitNode;
      second: SplitNode;
    };

/** Path from the root: 0 = first child, 1 = second child. */
export type SplitPath = number[];

/** Most terminals one tab can hold (each visible one uses a WebGL context). */
export const MAX_PANES = 8;
export const MIN_RATIO = 0.1;

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface PaneRect extends Rect {
  id: string;
}

export interface DividerRect {
  path: SplitPath;
  direction: SplitDirection;
  /** The area of the split this divider belongs to (for drag maths). */
  area: Rect;
  ratio: number;
}

export function leaf(id: string): SplitNode {
  return { kind: "leaf", id };
}

export function leaves(node: SplitNode): string[] {
  return node.kind === "leaf" ? [node.id] : [...leaves(node.first), ...leaves(node.second)];
}

export function contains(node: SplitNode, id: string): boolean {
  return leaves(node).includes(id);
}

/** Replace leaf `target` with a split of [target, newId]. Unknown target → unchanged. */
export function splitLeaf(
  node: SplitNode,
  target: string,
  newId: string,
  direction: SplitDirection,
): SplitNode {
  if (node.kind === "leaf") {
    return node.id === target
      ? { kind: "split", direction, ratio: 0.5, first: node, second: leaf(newId) }
      : node;
  }
  return {
    ...node,
    first: splitLeaf(node.first, target, newId, direction),
    second: splitLeaf(node.second, target, newId, direction),
  };
}

/** Remove a leaf; its sibling takes the parent's place. `null` if nothing is left. */
export function removeLeaf(node: SplitNode, id: string): SplitNode | null {
  if (node.kind === "leaf") return node.id === id ? null : node;
  const first = removeLeaf(node.first, id);
  const second = removeLeaf(node.second, id);
  if (!first) return second;
  if (!second) return first;
  return { ...node, first, second };
}

/** Rename a leaf (used when a pane is promoted). */
export function renameLeaf(node: SplitNode, from: string, to: string): SplitNode {
  if (node.kind === "leaf") return node.id === from ? leaf(to) : node;
  return {
    ...node,
    first: renameLeaf(node.first, from, to),
    second: renameLeaf(node.second, from, to),
  };
}

export function clampRatio(ratio: number): number {
  if (!Number.isFinite(ratio)) return 0.5;
  return Math.min(1 - MIN_RATIO, Math.max(MIN_RATIO, ratio));
}

export function setRatio(node: SplitNode, path: SplitPath, ratio: number): SplitNode {
  if (node.kind === "leaf") return node;
  const [head, ...rest] = path;
  if (head === undefined) return { ...node, ratio: clampRatio(ratio) };
  return head === 0
    ? { ...node, first: setRatio(node.first, rest, ratio) }
    : { ...node, second: setRatio(node.second, rest, ratio) };
}

/** Rectangles (fractions of the tab area) for panes and dividers. */
export function layoutRects(
  node: SplitNode,
  area: Rect = { x: 0, y: 0, w: 1, h: 1 },
  path: SplitPath = [],
): { panes: PaneRect[]; dividers: DividerRect[] } {
  if (node.kind === "leaf") return { panes: [{ id: node.id, ...area }], dividers: [] };
  const r = clampRatio(node.ratio);
  const [a, b] =
    node.direction === "row"
      ? [
          { ...area, w: area.w * r },
          { ...area, x: area.x + area.w * r, w: area.w * (1 - r) },
        ]
      : [
          { ...area, h: area.h * r },
          { ...area, y: area.y + area.h * r, h: area.h * (1 - r) },
        ];
  const first = layoutRects(node.first, a, [...path, 0]);
  const second = layoutRects(node.second, b, [...path, 1]);
  return {
    panes: [...first.panes, ...second.panes],
    dividers: [
      { path, direction: node.direction, area, ratio: r },
      ...first.dividers,
      ...second.dividers,
    ],
  };
}

/** Closest pane in a direction, by rectangle centres (for Alt+Arrow). */
export function neighbour(
  node: SplitNode,
  from: string,
  dir: "left" | "right" | "up" | "down",
): string | null {
  const { panes } = layoutRects(node);
  const cur = panes.find((p) => p.id === from);
  if (!cur) return null;
  const cx = cur.x + cur.w / 2;
  const cy = cur.y + cur.h / 2;
  const eps = 1e-9;
  let best: { id: string; d: number } | null = null;
  for (const p of panes) {
    if (p.id === from) continue;
    const ok =
      dir === "left"
        ? p.x + p.w <= cur.x + eps
        : dir === "right"
          ? p.x >= cur.x + cur.w - eps
          : dir === "up"
            ? p.y + p.h <= cur.y + eps
            : p.y >= cur.y + cur.h - eps;
    if (!ok) continue;
    const px = p.x + p.w / 2;
    const py = p.y + p.h / 2;
    const d = Math.hypot(px - cx, py - cy);
    if (!best || d < best.d) best = { id: p.id, d };
  }
  return best?.id ?? null;
}
