import { useState } from "react";
import { AlertTriangle, ClipboardPaste, Radio } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { withoutFinalNewline } from "@/lib/paste";
import { usePasteStore, type PasteRequest } from "@/stores/paste";

const PREVIEW_CHARS = 20_000;

/** Paste safety check: multi-line, large or suspicious pastes are shown first. */
export function PasteDialog() {
  const request = usePasteStore((s) => s.current);
  const answer = usePasteStore((s) => s.answer);
  return (
    <Dialog
      open={request !== null}
      onOpenChange={(open) => {
        if (!open) answer(null);
      }}
    >
      {request && <PasteReview key={request.id} request={request} onAnswer={answer} />}
    </Dialog>
  );
}

function PasteReview({
  request,
  onAnswer,
}: {
  request: PasteRequest;
  onAnswer: (text: string | null) => void;
}) {
  const { analysis: a, text, targets } = request;
  // The safe default: the last line waits for Enter instead of running.
  const [holdLast, setHoldLast] = useState(a.endsWithNewline);
  const preview = text.length > PREVIEW_CHARS ? `${text.slice(0, PREVIEW_CHARS)}\n…` : text;
  const lines = `${String(a.lines)} line${a.lines === 1 ? "" : "s"}`;

  return (
    <DialogContent
      className="sm:max-w-2xl"
      // Enter in the terminal must not confirm by accident: focus Cancel.
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        document.getElementById("paste-cancel")?.focus();
      }}
    >
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2">
          <ClipboardPaste className="size-4" /> Check this paste
        </DialogTitle>
        <DialogDescription>
          {lines}, {a.chars.toLocaleString()} characters.{" "}
          {a.multiLine
            ? "Every line ending in a line break runs as soon as it is pasted."
            : "Review it before it is sent."}
        </DialogDescription>
      </DialogHeader>

      {targets > 1 && (
        <p className="flex items-center gap-2 rounded-md border border-orange-500/50 bg-orange-500/10 px-3 py-2 text-sm">
          <Radio className="size-4 shrink-0 text-orange-400" />
          <span>
            Multi-exec is on: this goes to <strong>{targets} terminals</strong>.
          </span>
        </p>
      )}

      {(a.controlChars > 0 || a.risks.length > 0) && (
        <ul className="border-destructive/40 bg-destructive/10 space-y-1 rounded-md border px-3 py-2 text-sm">
          {a.controlChars > 0 && (
            <li className="flex gap-2">
              <AlertTriangle className="text-destructive mt-0.5 size-4 shrink-0" />
              <span>
                {a.controlChars} hidden control character{a.controlChars === 1 ? "" : "s"} found
                (often from web pages). They will be removed.
              </span>
            </li>
          )}
          {a.risks.map((r) => (
            <li key={r} className="flex gap-2">
              <AlertTriangle className="text-destructive mt-0.5 size-4 shrink-0" />
              <span>{r}</span>
            </li>
          ))}
        </ul>
      )}

      <pre
        aria-label="Text to paste"
        className="bg-muted/50 max-h-72 overflow-auto rounded-md border p-3 font-mono text-xs leading-relaxed break-all whitespace-pre-wrap"
      >
        {preview}
      </pre>

      {a.endsWithNewline && (
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            className="accent-brand size-4"
            checked={holdLast}
            onChange={(e) => {
              setHoldLast(e.target.checked);
            }}
          />
          Don't run the last line (remove the final line break)
        </label>
      )}

      <DialogFooter>
        <Button
          id="paste-cancel"
          variant="outline"
          onClick={() => {
            onAnswer(null);
          }}
        >
          Cancel
        </Button>
        <Button
          onClick={() => {
            onAnswer(holdLast ? withoutFinalNewline(text) : text);
          }}
        >
          <ClipboardPaste /> Paste {targets > 1 ? `to ${String(targets)} terminals` : lines}
        </Button>
      </DialogFooter>
    </DialogContent>
  );
}
