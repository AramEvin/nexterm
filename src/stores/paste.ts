/**
 * Paste safety check queue (Phase 6). A terminal asks `review()`; pastes that
 * need a look open the dialog, everything else resolves at once.
 */
import { create } from "zustand";

import { analyzePaste, stripControlChars, type PasteAnalysis } from "@/lib/paste";

export interface PasteRequest {
  id: number;
  text: string;
  analysis: PasteAnalysis;
  /** Terminals that will receive it (more than 1 in multi-exec). */
  targets: number;
  resolve: (text: string | null) => void;
}

interface PasteState {
  current: PasteRequest | null;
  review: (text: string, targets: number) => Promise<string | null>;
  /** Answer the open request: the text to paste, or `null` to cancel. */
  answer: (text: string | null) => void;
}

let nextId = 1;

export const usePasteStore = create<PasteState>()((set, get) => ({
  current: null,
  review: (text, targets) => {
    const analysis = analyzePaste(text);
    // In multi-exec even a one-liner goes to several servers: always ask.
    if (!analysis.needsReview && targets <= 1) return Promise.resolve(text);
    // One dialog at a time: a second paste while one is open is dropped.
    if (get().current) return Promise.resolve(null);
    return new Promise((resolve) => {
      set({
        current: {
          id: nextId++,
          text: analysis.controlChars > 0 ? stripControlChars(text) : text,
          analysis,
          targets,
          resolve,
        },
      });
    });
  },
  answer: (text) => {
    const req = get().current;
    if (!req) return;
    set({ current: null });
    req.resolve(text);
  },
}));
