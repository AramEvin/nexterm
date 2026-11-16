import { useState } from "react";
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
import { IpcError } from "@/lib/ipc";
import { useDialogStore } from "@/stores/dialogs";

/** Generic "are you sure?" dialog driven by `useDialogStore().askConfirm`. */
export function ConfirmDialog() {
  const request = useDialogStore((s) => s.confirm);
  const close = useDialogStore((s) => s.closeConfirm);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function confirm() {
    if (!request) return;
    setBusy(true);
    setError(null);
    try {
      await request.onConfirm();
      close();
    } catch (err) {
      setError(err instanceof IpcError ? err.message : "The action failed.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      open={request !== null}
      onOpenChange={(next) => {
        if (!next && !busy) {
          setError(null);
          close();
        }
      }}
    >
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle>{request?.title}</DialogTitle>
          <DialogDescription>{request?.description}</DialogDescription>
        </DialogHeader>
        {error && (
          <p role="alert" className="text-destructive text-sm">
            {error}
          </p>
        )}
        <DialogFooter>
          <Button
            variant="outline"
            onClick={close}
            disabled={busy}
            autoFocus={request?.destructive === true}
          >
            Cancel
          </Button>
          <Button
            variant={request?.destructive ? "destructive" : "default"}
            onClick={() => void confirm()}
            disabled={busy}
            autoFocus={request?.destructive !== true}
          >
            {busy && <Loader2 className="animate-spin" />}
            {request?.confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
