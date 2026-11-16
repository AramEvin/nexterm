import { useEffect, useRef, useState, type SyntheticEvent } from "react";
import { Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { formatMode, formatOctal, parseOctal } from "@/lib/files";
import { IpcError } from "@/lib/ipc";
import { useFileDialogStore, type ChmodRequest, type NameRequest } from "@/stores/fileDialogs";

/** Name input for "New folder" and "Rename". Selects the name without its extension. */
function NameDialog() {
  const request = useFileDialogStore((s) => s.name);
  const close = useFileDialogStore((s) => s.close);
  const [busy, setBusy] = useState(false);
  return (
    <Dialog
      open={request !== null}
      onOpenChange={(next) => {
        if (!next && !busy) close("name");
      }}
    >
      <DialogContent className="sm:max-w-sm">
        {request && (
          <NameForm
            key={`${request.title}:${request.initial}`}
            request={request}
            busy={busy}
            setBusy={setBusy}
            onClose={() => {
              close("name");
            }}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

function NameForm({
  request,
  busy,
  setBusy,
  onClose,
}: {
  request: NameRequest;
  busy: boolean;
  setBusy: (busy: boolean) => void;
  onClose: () => void;
}) {
  const [value, setValue] = useState(request.initial);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    // Select "report" in "report.pdf" so typing keeps the extension.
    const id = requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.focus();
      const dot = request.initial.lastIndexOf(".");
      el.setSelectionRange(0, dot > 0 ? dot : request.initial.length);
    });
    return () => {
      cancelAnimationFrame(id);
    };
  }, [request]);

  async function submit(e: SyntheticEvent) {
    e.preventDefault();
    if (!value.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await request.onSubmit(value);
      onClose();
    } catch (err) {
      setError(err instanceof IpcError ? err.message : "The action failed.");
      inputRef.current?.focus();
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={(e) => void submit(e)} className="grid gap-4">
      <DialogHeader>
        <DialogTitle>{request.title}</DialogTitle>
      </DialogHeader>
      <div className="grid gap-2">
        <Label htmlFor="fm-name">{request.label}</Label>
        <Input
          id="fm-name"
          ref={inputRef}
          value={value}
          maxLength={255}
          spellCheck={false}
          aria-invalid={error !== null}
          onChange={(e) => {
            setValue(e.target.value);
          }}
        />
        {error && (
          <p role="alert" className="text-destructive text-sm">
            {error}
          </p>
        )}
      </div>
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button type="submit" disabled={busy || !value.trim()}>
          {busy && <Loader2 className="animate-spin" />}
          {request.confirmLabel}
        </Button>
      </DialogFooter>
    </form>
  );
}

const WHO = [
  { label: "Owner", shift: 6 },
  { label: "Group", shift: 3 },
  { label: "Others", shift: 0 },
] as const;
const WHAT = [
  { label: "Read", bit: 4 },
  { label: "Write", bit: 2 },
  { label: "Execute", bit: 1 },
] as const;

/** Permissions as a checkbox grid plus an octal field, kept in sync. */
function ChmodDialog() {
  const request = useFileDialogStore((s) => s.chmod);
  const close = useFileDialogStore((s) => s.close);
  const [busy, setBusy] = useState(false);
  return (
    <Dialog
      open={request !== null}
      onOpenChange={(next) => {
        if (!next && !busy) close("chmod");
      }}
    >
      <DialogContent className="sm:max-w-sm">
        {request && (
          <ChmodForm
            key={`${request.what}:${String(request.initialMode)}`}
            request={request}
            busy={busy}
            setBusy={setBusy}
            onClose={() => {
              close("chmod");
            }}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

function ChmodForm({
  request,
  busy,
  setBusy,
  onClose,
}: {
  request: ChmodRequest;
  busy: boolean;
  setBusy: (busy: boolean) => void;
  onClose: () => void;
}) {
  const [mode, setMode] = useState(request.initialMode & 0o7777);
  const [octal, setOctal] = useState(formatOctal(request.initialMode));
  const [error, setError] = useState<string | null>(null);

  const toggle = (flag: number) => {
    const next = mode ^ flag;
    setMode(next);
    setOctal(formatOctal(next));
  };

  async function submit(e: SyntheticEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await request.onSubmit(mode);
      onClose();
    } catch (err) {
      setError(err instanceof IpcError ? err.message : "Changing permissions failed.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={(e) => void submit(e)} className="grid gap-4">
      <DialogHeader>
        <DialogTitle>Permissions of {request.what}</DialogTitle>
        <DialogDescription>Applies to the selected items only (not recursive).</DialogDescription>
      </DialogHeader>
      <table className="text-sm">
        <thead>
          <tr className="text-muted-foreground">
            <th />
            {WHAT.map((w) => (
              <th key={w.label} className="px-2 py-1 font-normal">
                {w.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {WHO.map((who) => (
            <tr key={who.label}>
              <td className="py-1 pr-2">{who.label}</td>
              {WHAT.map((w) => {
                const flag = w.bit << who.shift;
                return (
                  <td key={w.label} className="text-center">
                    <input
                      type="checkbox"
                      aria-label={`${who.label} ${w.label}`}
                      className="accent-brand size-4"
                      checked={(mode & flag) !== 0}
                      onChange={() => {
                        toggle(flag);
                      }}
                    />
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
      <div className="flex items-center gap-3">
        <Label htmlFor="fm-octal">Octal</Label>
        <Input
          id="fm-octal"
          value={octal}
          maxLength={4}
          className="w-20 font-mono"
          aria-invalid={parseOctal(octal) === null}
          onChange={(e) => {
            setOctal(e.target.value);
            const parsed = parseOctal(e.target.value);
            if (parsed !== null) setMode(parsed);
          }}
        />
        <code className="text-muted-foreground font-mono text-sm">{formatMode(mode)}</code>
      </div>
      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button type="submit" disabled={busy || parseOctal(octal) === null}>
          {busy && <Loader2 className="animate-spin" />}
          Apply
        </Button>
      </DialogFooter>
    </form>
  );
}

/** Some items already exist in the target folder. Default focus: the safe choice. */
function ConflictDialog() {
  const request = useFileDialogStore((s) => s.conflict);
  const close = useFileDialogStore((s) => s.close);
  const choose = (choice: "overwrite" | "skip" | "rename" | null) => {
    request?.onChoose(choice);
    close("conflict");
  };
  const names = request?.names ?? [];
  const shown = names.slice(0, 8);

  return (
    <Dialog
      open={request !== null}
      onOpenChange={(next) => {
        if (!next) choose(null);
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            {names.length === 1
              ? `"${names[0] ?? ""}" already exists`
              : `${String(names.length)} items already exist`}
          </DialogTitle>
          <DialogDescription className="break-all">in {request?.targetLabel}</DialogDescription>
        </DialogHeader>
        {names.length > 1 && (
          <ul className="bg-muted/40 max-h-40 overflow-auto rounded-md border p-2 font-mono text-xs">
            {shown.map((n) => (
              <li key={n} className="truncate">
                {n}
              </li>
            ))}
            {names.length > shown.length && (
              <li className="text-muted-foreground">
                …and {String(names.length - shown.length)} more
              </li>
            )}
          </ul>
        )}
        <DialogFooter className="gap-2 sm:justify-between">
          <Button
            variant="ghost"
            onClick={() => {
              choose(null);
            }}
          >
            Cancel
          </Button>
          <div className="flex gap-2">
            <Button
              variant="outline"
              onClick={() => {
                choose("skip");
              }}
            >
              Skip
            </Button>
            <Button
              autoFocus
              variant="outline"
              onClick={() => {
                choose("rename");
              }}
            >
              Keep both
            </Button>
            <Button
              variant="destructive"
              onClick={() => {
                choose("overwrite");
              }}
            >
              Overwrite
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function FileDialogs() {
  return (
    <>
      <NameDialog />
      <ChmodDialog />
      <ConflictDialog />
    </>
  );
}
