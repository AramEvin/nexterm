import { useState, type SyntheticEvent } from "react";
import { KeyRound, Loader2, Lock, ShieldCheck } from "lucide-react";

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
import { IpcError } from "@/lib/ipc";
import { MIN_MASTER_PASSWORD_LENGTH as MIN_LENGTH, passwordStrength } from "@/lib/password";
import { cn } from "@/lib/utils";
import { useDialogStore } from "@/stores/dialogs";
import { useVaultStore } from "@/stores/vault";

function StrengthMeter({ password }: { password: string }) {
  const { score, label } = passwordStrength(password);
  return (
    <div className="grid gap-1" aria-live="polite">
      <div className="flex gap-1">
        {[1, 2, 3].map((i) => (
          <span
            key={i}
            className={cn(
              "h-1 flex-1 rounded-full",
              score >= i
                ? score === 1
                  ? "bg-destructive"
                  : score === 2
                    ? "bg-amber-400"
                    : "bg-emerald-500"
                : "bg-muted",
            )}
          />
        ))}
      </div>
      <span className="text-muted-foreground text-xs">{password ? label : " "}</span>
    </div>
  );
}

function RememberCheckbox({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label className="flex items-start gap-2 text-sm">
      <input
        type="checkbox"
        className="accent-brand mt-0.5 size-4"
        checked={checked}
        disabled={disabled}
        onChange={(e) => {
          onChange(e.target.checked);
        }}
      />
      <span>
        Remember on this device
        <span className="text-muted-foreground block text-xs">
          Unlocks automatically when NexTerm starts (key kept in Windows Credential Manager). Any
          program running as you could read it, so leave this off on shared machines.
        </span>
      </span>
    </label>
  );
}

/** First-run wizard: create the master password. */
export function VaultSetupDialog() {
  const open = useDialogStore((s) => s.vaultSetup);
  const close = useDialogStore((s) => s.closeVaultSetup);
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent className="sm:max-w-md">{open && <SetupForm onDone={close} />}</DialogContent>
    </Dialog>
  );
}

function SetupForm({ onDone }: { onDone: () => void }) {
  const setup = useVaultStore((s) => s.setup);
  const [pw, setPw] = useState("");
  const [confirm, setConfirm] = useState("");
  const [remember, setRemember] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onSubmit(e: SyntheticEvent<HTMLFormElement>) {
    e.preventDefault();
    if (pw.length < MIN_LENGTH) {
      setError(`Use at least ${MIN_LENGTH} characters.`);
      return;
    }
    if (pw !== confirm) {
      setError("The passwords don't match.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await setup(pw, remember);
      onDone();
    } catch (err) {
      setError(err instanceof IpcError ? err.message : "Could not create the vault.");
    } finally {
      setBusy(false);
      setPw("");
      setConfirm("");
    }
  }

  return (
    <form onSubmit={(e) => void onSubmit(e)} className="grid gap-4" autoComplete="off">
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2">
          <ShieldCheck className="text-brand size-5" /> Set up the vault
        </DialogTitle>
        <DialogDescription>
          The vault stores passwords and SSH keys encrypted on this computer. Choose a master
          password. It is never stored, and <strong>it can't be recovered</strong> if you forget it.
        </DialogDescription>
      </DialogHeader>
      <div className="grid gap-1.5">
        <Label htmlFor="vs-pw">Master password</Label>
        <Input
          id="vs-pw"
          type="password"
          autoFocus
          autoComplete="new-password"
          value={pw}
          disabled={busy}
          onChange={(e) => {
            setPw(e.target.value);
          }}
        />
        <StrengthMeter password={pw} />
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="vs-confirm">Repeat master password</Label>
        <Input
          id="vs-confirm"
          type="password"
          autoComplete="new-password"
          value={confirm}
          disabled={busy}
          onChange={(e) => {
            setConfirm(e.target.value);
          }}
        />
      </div>
      <RememberCheckbox checked={remember} onChange={setRemember} disabled={busy} />
      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onDone} disabled={busy}>
          Later
        </Button>
        <Button type="submit" disabled={busy}>
          {busy ? <Loader2 className="animate-spin" /> : <KeyRound />}
          {busy ? "Creating…" : "Create vault"}
        </Button>
      </DialogFooter>
    </form>
  );
}

/** Unlock screen (also opened when a connection needs a saved credential). */
export function UnlockDialog() {
  const target = useDialogStore((s) => s.unlock);
  const close = useDialogStore((s) => s.closeUnlock);
  return (
    <Dialog
      open={target !== null}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent className="sm:max-w-sm">
        {target && (
          <UnlockForm
            onDone={(unlocked) => {
              close();
              if (unlocked) target.onUnlocked?.();
            }}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

function UnlockForm({ onDone }: { onDone: (unlocked: boolean) => void }) {
  const unlock = useVaultStore((s) => s.unlock);
  const status = useVaultStore((s) => s.status);
  const [pw, setPw] = useState("");
  const [remember, setRemember] = useState(status?.remembered ?? false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onSubmit(e: SyntheticEvent<HTMLFormElement>) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await unlock(pw, remember);
      onDone(true);
    } catch (err) {
      setError(err instanceof IpcError ? err.message : "Could not unlock the vault.");
    } finally {
      setBusy(false);
      setPw("");
    }
  }

  return (
    <form onSubmit={(e) => void onSubmit(e)} className="grid gap-4" autoComplete="off">
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2">
          <Lock className="text-brand size-5" /> Unlock the vault
        </DialogTitle>
        <DialogDescription>Enter your master password to use saved credentials.</DialogDescription>
      </DialogHeader>
      <div className="grid gap-1.5">
        <Label htmlFor="vu-pw">Master password</Label>
        <Input
          id="vu-pw"
          type="password"
          autoFocus
          autoComplete="current-password"
          value={pw}
          disabled={busy}
          onChange={(e) => {
            setPw(e.target.value);
          }}
        />
      </div>
      <RememberCheckbox checked={remember} onChange={setRemember} disabled={busy} />
      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}
      <DialogFooter>
        <Button
          type="button"
          variant="outline"
          onClick={() => {
            onDone(false);
          }}
          disabled={busy}
        >
          Not now
        </Button>
        <Button type="submit" disabled={busy || !pw}>
          {busy && <Loader2 className="animate-spin" />}
          {busy ? "Unlocking…" : "Unlock"}
        </Button>
      </DialogFooter>
    </form>
  );
}
