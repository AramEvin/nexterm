import { useState, type SyntheticEvent } from "react";
import { Check, Copy, Fingerprint, KeySquare, ShieldAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import { sha256Hex } from "@/lib/fingerprint";
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
import { useDialogStore } from "@/stores/dialogs";
import { usePromptStore } from "@/stores/prompts";
import type { SshPrompt } from "@/types/ipc";

type HostKeyPrompt = Extract<SshPrompt, { type: "hostKey" }>;
type KbdPrompt = Extract<SshPrompt, { type: "keyboardInteractive" }>;

/** Shows the oldest pending prompt from Rust (host key or 2FA). */
export function PromptDialogs() {
  const current = usePromptStore((s) => s.queue[0]);
  const respond = usePromptStore((s) => s.respond);
  if (!current) return null;
  return current.type === "hostKey" ? (
    <HostKeyDialog key={current.promptId} prompt={current} respond={respond} />
  ) : (
    <KbdDialog key={current.promptId} prompt={current} respond={respond} />
  );
}

type Respond = ReturnType<typeof usePromptStore.getState>["respond"];

/** Unknown server key: show the fingerprint; the user decides. */
function HostKeyDialog({ prompt, respond }: { prompt: HostKeyPrompt; respond: Respond }) {
  const decide = (decision: "acceptOnce" | "acceptAndSave" | "reject") =>
    void respond(prompt.promptId, { type: "hostKey", decision });
  return (
    <Dialog
      open
      onOpenChange={(next) => {
        if (!next) decide("reject");
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Fingerprint className="text-brand size-5" />{" "}
            {prompt.kind === "tls" ? "Untrusted server certificate" : "Unknown server"}
          </DialogTitle>
          {prompt.kind === "tls" ? (
            <DialogDescription>
              The FTPS server{" "}
              <strong className="text-foreground">
                {prompt.host}:{prompt.port}
              </strong>{" "}
              uses a certificate that is not signed by a trusted authority (self-signed or an
              internal CA). Compare its SHA-256 fingerprint with the one your administrator gave you
              (on the server: <code>openssl x509 -in cert.pem -noout -fingerprint -sha256</code>
              ).
            </DialogDescription>
          ) : (
            <DialogDescription>
              NexTerm has not connected to{" "}
              <strong className="text-foreground">
                {prompt.host}:{prompt.port}
              </strong>{" "}
              before. Compare the fingerprint with the one your administrator gave you (on the
              server: <code>ssh-keygen -lf /etc/ssh/ssh_host_*_key.pub</code>).
            </DialogDescription>
          )}
        </DialogHeader>
        <div className="bg-muted/50 grid gap-1 rounded-md border p-3">
          <span className="text-muted-foreground text-xs">{prompt.keyType}</span>
          <code className="text-sm break-all select-all">{prompt.fingerprint}</code>
          {prompt.kind === "tls" && sha256Hex(prompt.fingerprint) && (
            <>
              <span className="text-muted-foreground mt-1 text-xs">Same, in OpenSSL's format</span>
              <code className="text-xs break-all select-all">{sha256Hex(prompt.fingerprint)}</code>
            </>
          )}
        </div>
        {prompt.otherKnownTypes.length > 0 && (
          <p className="text-sm text-amber-400">
            This server previously used a different key type ({prompt.otherKnownTypes.join(", ")}
            ). That can be harmless (new algorithm), but verify the fingerprint.
          </p>
        )}
        <DialogFooter className="gap-2 sm:justify-between">
          <Button
            variant="outline"
            autoFocus
            onClick={() => {
              decide("reject");
            }}
          >
            Reject
          </Button>
          <div className="flex gap-2">
            <Button
              variant="secondary"
              onClick={() => {
                decide("acceptOnce");
              }}
            >
              Accept once
            </Button>
            <Button
              onClick={() => {
                decide("acceptAndSave");
              }}
            >
              Accept &amp; save
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Keyboard-interactive round (OTP, 2FA, PAM questions). */
function KbdDialog({ prompt, respond }: { prompt: KbdPrompt; respond: Respond }) {
  const [answers, setAnswers] = useState<string[]>(() => prompt.fields.map(() => ""));
  const cancel = () => void respond(prompt.promptId, { type: "cancel" });

  function onSubmit(e: SyntheticEvent<HTMLFormElement>) {
    e.preventDefault();
    void respond(prompt.promptId, { type: "answers", answers });
    setAnswers(prompt.fields.map(() => ""));
  }

  return (
    <Dialog
      open
      onOpenChange={(next) => {
        if (!next) cancel();
      }}
    >
      <DialogContent className="sm:max-w-md">
        <form onSubmit={onSubmit} className="grid gap-4" autoComplete="off">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <KeySquare className="text-brand size-5" />
              {prompt.name || `Sign in to ${prompt.host}`}
            </DialogTitle>
            <DialogDescription className="whitespace-pre-line">
              {prompt.instructions || "The server asks for more information."}
            </DialogDescription>
          </DialogHeader>
          {prompt.fields.map((field, i) => (
            <div key={i} className="grid gap-1.5">
              <Label htmlFor={`kbd-${String(i)}`}>{field.text.trim() || "Answer"}</Label>
              <Input
                id={`kbd-${String(i)}`}
                type={field.echo ? "text" : "password"}
                autoFocus={i === 0}
                autoComplete="one-time-code"
                value={answers[i] ?? ""}
                onChange={(e) => {
                  const next = [...answers];
                  next[i] = e.target.value;
                  setAnswers(next);
                }}
              />
            </div>
          ))}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={cancel}>
              Cancel
            </Button>
            <Button type="submit">Continue</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/**
 * A changed host key blocked the connection. There is no "connect anyway":
 * the only way forward is to remove the stored key after verifying, which
 * requires typing the host name.
 */
export function HostKeyChangedDialog() {
  const info = useDialogStore((s) => s.hostKeyChanged);
  const close = useDialogStore((s) => s.closeHostKeyChanged);
  const inOpenSsh = info?.message.includes(".ssh/known_hosts") ?? false;
  const knownHostsName =
    info && info.port !== 22 ? `[${info.host}]:${String(info.port)}` : info?.host;

  return (
    <Dialog
      open={info !== null}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent className="border-destructive/60 sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="text-destructive flex items-center gap-2">
            <ShieldAlert className="size-6" /> Server identity changed: connection blocked
          </DialogTitle>
        </DialogHeader>
        <p className="text-sm whitespace-pre-line">{info?.message}</p>
        <div className="bg-muted/40 text-muted-foreground rounded-md border p-3 text-sm leading-relaxed">
          {inOpenSsh ? (
            <>
              Only after verifying the new fingerprint, remove the old key from{" "}
              <code className="text-foreground">~/.ssh/known_hosts</code> in PowerShell:
              <CopyCommand command={`ssh-keygen -R ${knownHostsName ?? ""}`} />
            </>
          ) : (
            <>
              Only after verifying the new fingerprint, remove the old key in{" "}
              <span className="text-foreground">Credentials → Known hosts</span>.
            </>
          )}
        </div>
        <DialogFooter>
          <Button autoFocus onClick={close}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** A command in monospace with a Copy button. */
function CopyCommand({ command }: { command: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <span className="mt-1 flex items-center gap-2">
      <code className="bg-background text-foreground min-w-0 flex-1 truncate rounded px-2 py-1 select-all">
        {command}
      </code>
      <Button
        type="button"
        size="sm"
        variant="outline"
        onClick={() => {
          void navigator.clipboard.writeText(command).then(() => {
            setCopied(true);
            setTimeout(() => {
              setCopied(false);
            }, 1500);
          });
        }}
      >
        {copied ? <Check /> : <Copy />} {copied ? "Copied" : "Copy"}
      </Button>
    </span>
  );
}

/** Simple message box. */
export function AlertDialog() {
  const alert = useDialogStore((s) => s.alert);
  const close = useDialogStore((s) => s.closeAlert);
  return (
    <Dialog
      open={alert !== null}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{alert?.title}</DialogTitle>
          <DialogDescription className="whitespace-pre-line">{alert?.message}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button autoFocus onClick={close}>
            OK
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
