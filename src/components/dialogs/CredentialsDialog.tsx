import { useEffect, useState, type SyntheticEvent } from "react";
import {
  FileKey2,
  KeyRound,
  Loader2,
  Lock,
  Pencil,
  Plus,
  ServerCog,
  ShieldCheck,
  Trash2,
} from "lucide-react";

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
import { folderSetDefaultCredential, IpcError, knownHostsList, knownHostsRemove } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useDialogStore } from "@/stores/dialogs";
import { useLibraryStore } from "@/stores/library";
import { useVaultStore } from "@/stores/vault";
import type { Credential, KnownHost } from "@/types/ipc";

const selectClass =
  "border-input bg-background dark:bg-input/30 h-9 rounded-md border px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]";

const AUTO_LOCK_OPTIONS = [5, 15, 30, 60, 240, 0];

function errorText(err: unknown, fallback: string): string {
  return err instanceof IpcError ? err.message : fallback;
}

type Section = "credentials" | "vault" | "hosts";

/** Credentials, vault settings and trusted host keys in one place. */
export function CredentialsDialog() {
  const open = useDialogStore((s) => s.credentials);
  const close = useDialogStore((s) => s.closeCredentials);
  const [section, setSection] = useState<Section>("credentials");

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ShieldCheck className="text-brand size-5" /> Vault &amp; credentials
          </DialogTitle>
          <DialogDescription>
            Secrets are encrypted with your master password and never leave the app&apos;s core.
          </DialogDescription>
        </DialogHeader>
        <div className="flex gap-1 border-b" role="tablist">
          {(
            [
              ["credentials", "Credentials"],
              ["vault", "Vault settings"],
              ["hosts", "Known hosts"],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={section === id}
              onClick={() => {
                setSection(id);
              }}
              className={cn(
                "-mb-px border-b-2 px-3 py-1.5 text-sm",
                section === id
                  ? "border-brand text-foreground"
                  : "text-muted-foreground hover:text-foreground border-transparent",
              )}
            >
              {label}
            </button>
          ))}
        </div>
        {open && section === "credentials" && <CredentialsSection />}
        {open && section === "vault" && <VaultSection />}
        {open && section === "hosts" && <KnownHostsSection />}
      </DialogContent>
    </Dialog>
  );
}

/** Shown instead of the content when the vault isn't usable yet. */
function VaultGate({ children }: { children: React.ReactNode }) {
  const status = useVaultStore((s) => s.status);
  const openVaultSetup = useDialogStore((s) => s.openVaultSetup);
  const openUnlock = useDialogStore((s) => s.openUnlock);
  if (!status) return null;
  if (!status.initialized) {
    return (
      <div className="flex flex-col items-center gap-3 py-8 text-center text-sm">
        <ShieldCheck className="text-brand size-8" />
        <p>Set up the vault to save passwords and SSH keys.</p>
        <Button onClick={openVaultSetup}>
          <KeyRound /> Set up vault
        </Button>
      </div>
    );
  }
  if (!status.unlocked) {
    return (
      <div className="flex flex-col items-center gap-3 py-8 text-center text-sm">
        <Lock className="text-muted-foreground size-8" />
        <p>The vault is locked.</p>
        <Button
          onClick={() => {
            openUnlock();
          }}
        >
          Unlock
        </Button>
      </div>
    );
  }
  return <>{children}</>;
}

type Editing =
  | { mode: "none" }
  | { mode: "password" }
  | { mode: "key" }
  | { mode: "edit"; credential: Credential };

function CredentialsSection() {
  const credentials = useVaultStore((s) => s.credentials);
  const deleteCredential = useVaultStore((s) => s.deleteCredential);
  const askConfirm = useDialogStore((s) => s.askConfirm);
  const reloadLibrary = useLibraryStore((s) => s.load);
  const [editing, setEditing] = useState<Editing>({ mode: "none" });

  return (
    <VaultGate>
      <div className="grid gap-3">
        <div className="flex gap-2">
          <Button
            size="sm"
            onClick={() => {
              setEditing({ mode: "password" });
            }}
          >
            <Plus /> Add password
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              setEditing({ mode: "key" });
            }}
          >
            <FileKey2 /> Import private key
          </Button>
        </div>

        {editing.mode !== "none" && (
          <CredentialForm
            key={editing.mode === "edit" ? editing.credential.id : editing.mode}
            editing={editing}
            onDone={() => {
              setEditing({ mode: "none" });
            }}
          />
        )}

        {credentials.length === 0 ? (
          <p className="text-muted-foreground py-6 text-center text-sm">
            No saved credentials yet.
          </p>
        ) : (
          <ul className="divide-y rounded-md border">
            {credentials.map((c) => (
              <li key={c.id} className="flex items-center gap-3 px-3 py-2 text-sm">
                {c.kind === "key" ? (
                  <FileKey2 className="text-brand size-4 shrink-0" />
                ) : (
                  <KeyRound className="text-brand size-4 shrink-0" />
                )}
                <div className="min-w-0 flex-1">
                  <div className="truncate font-medium">{c.name}</div>
                  <div className="text-muted-foreground truncate text-xs">
                    {[
                      c.kind === "key"
                        ? `${c.keyAlgorithm ?? "key"} ${c.keyFingerprint ?? ""}`
                        : "Password",
                      c.username && `user ${c.username}`,
                      c.usedBy > 0 && `used by ${String(c.usedBy)}`,
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </div>
                </div>
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label={`Edit ${c.name}`}
                  onClick={() => {
                    setEditing({ mode: "edit", credential: c });
                  }}
                >
                  <Pencil />
                </Button>
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label={`Delete ${c.name}`}
                  onClick={() => {
                    askConfirm({
                      title: "Delete credential?",
                      description:
                        c.usedBy > 0
                          ? `"${c.name}" is used by ${String(c.usedBy)} session(s) or folder(s). They will ask for a password (or fail for key logins) until you link another credential.`
                          : `"${c.name}" will be deleted from the vault.`,
                      confirmLabel: "Delete",
                      destructive: true,
                      onConfirm: async () => {
                        await deleteCredential(c.id);
                        await reloadLibrary();
                      },
                    });
                  }}
                >
                  <Trash2 />
                </Button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </VaultGate>
  );
}

function CredentialForm({ editing, onDone }: { editing: Editing; onDone: () => void }) {
  const createPassword = useVaultStore((s) => s.createPassword);
  const importKey = useVaultStore((s) => s.importKey);
  const updateCredential = useVaultStore((s) => s.updateCredential);
  const existing = editing.mode === "edit" ? editing.credential : null;
  const isKey = editing.mode === "key" || existing?.kind === "key";

  const [name, setName] = useState(existing?.name ?? "");
  const [username, setUsername] = useState(existing?.username ?? "");
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onSubmit(e: SyntheticEvent<HTMLFormElement>) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      if (existing) {
        await updateCredential(existing.id, name, username, secret || null);
        onDone();
      } else if (isKey) {
        // Rust opens the file picker; the key never passes through the UI.
        const cred = await importKey(name, username, secret || null);
        if (cred) onDone();
      } else {
        if (!secret) {
          setError("Enter the password to store.");
          return;
        }
        await createPassword(name, username, secret);
        onDone();
      }
    } catch (err) {
      setError(errorText(err, "Could not save the credential."));
    } finally {
      setBusy(false);
      setSecret("");
    }
  }

  const title = existing
    ? `Edit "${existing.name}"`
    : isKey
      ? "Import a private key (OpenSSH, PEM or PuTTY .ppk)"
      : "Add a password";

  return (
    <form
      onSubmit={(e) => void onSubmit(e)}
      className="bg-muted/30 grid gap-3 rounded-md border p-3"
      autoComplete="off"
    >
      <p className="text-sm font-medium">{title}</p>
      <div className="grid grid-cols-2 gap-3">
        <div className="grid gap-1.5">
          <Label htmlFor="cf-name">Name</Label>
          <Input
            id="cf-name"
            autoFocus
            placeholder={isKey ? "Defaults to the file name" : "e.g. prod root"}
            value={name}
            maxLength={128}
            onChange={(e) => {
              setName(e.target.value);
            }}
          />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="cf-user">Username (optional)</Label>
          <Input
            id="cf-user"
            placeholder="Used if the session has none"
            value={username}
            spellCheck={false}
            onChange={(e) => {
              setUsername(e.target.value);
            }}
          />
        </div>
      </div>
      {!(existing && isKey) && (
        <div className="grid gap-1.5">
          <Label htmlFor="cf-secret">
            {isKey
              ? "Key passphrase (if the key has one)"
              : existing
                ? "New password (leave empty to keep the current one)"
                : "Password"}
          </Label>
          <Input
            id="cf-secret"
            type="password"
            autoComplete="new-password"
            value={secret}
            onChange={(e) => {
              setSecret(e.target.value);
            }}
          />
        </div>
      )}
      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <Button type="button" variant="outline" size="sm" onClick={onDone} disabled={busy}>
          Cancel
        </Button>
        <Button type="submit" size="sm" disabled={busy}>
          {busy && <Loader2 className="animate-spin" />}
          {existing ? "Save" : isKey ? "Choose key file…" : "Save password"}
        </Button>
      </div>
    </form>
  );
}

function VaultSection() {
  const status = useVaultStore((s) => s.status);
  const lock = useVaultStore((s) => s.lock);
  const setAutoLock = useVaultStore((s) => s.setAutoLock);
  const forgetDevice = useVaultStore((s) => s.forgetDevice);
  const changePassword = useVaultStore((s) => s.changePassword);
  const [oldPw, setOldPw] = useState("");
  const [newPw, setNewPw] = useState("");
  const [newPw2, setNewPw2] = useState("");
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  async function onChangePassword(e: SyntheticEvent<HTMLFormElement>) {
    e.preventDefault();
    if (newPw !== newPw2) {
      setMessage({ ok: false, text: "The new passwords don't match." });
      return;
    }
    setBusy(true);
    try {
      await changePassword(oldPw, newPw);
      setMessage({ ok: true, text: "Master password changed." });
    } catch (err) {
      setMessage({ ok: false, text: errorText(err, "Could not change the password.") });
    } finally {
      setBusy(false);
      setOldPw("");
      setNewPw("");
      setNewPw2("");
    }
  }

  return (
    <VaultGate>
      <div className="grid gap-5 text-sm">
        <div className="flex items-center justify-between gap-3">
          <div>
            <p className="font-medium">Lock now</p>
            <p className="text-muted-foreground text-xs">
              Removes the key from memory. Open tabs stay connected.
            </p>
          </div>
          <Button size="sm" variant="secondary" onClick={() => void lock()}>
            <Lock /> Lock
          </Button>
        </div>

        <div className="flex items-center justify-between gap-3">
          <div>
            <p className="font-medium">Auto-lock</p>
            <p className="text-muted-foreground text-xs">Lock after this long without activity.</p>
          </div>
          <select
            className={selectClass}
            value={status?.autoLockMinutes ?? 15}
            onChange={(e) => void setAutoLock(Number(e.target.value))}
            aria-label="Auto-lock after"
          >
            {AUTO_LOCK_OPTIONS.map((m) => (
              <option key={m} value={m}>
                {m === 0 ? "Never" : m < 60 ? `${String(m)} minutes` : `${String(m / 60)} hour(s)`}
              </option>
            ))}
          </select>
        </div>

        <div className="flex items-center justify-between gap-3">
          <div>
            <p className="font-medium">Remembered on this device</p>
            <p className="text-muted-foreground text-xs">
              {status?.remembered
                ? "Unlocks automatically at start-up (Windows Credential Manager)."
                : "Off. Turn it on by ticking “Remember on this device” when unlocking."}
            </p>
          </div>
          {status?.remembered && (
            <Button size="sm" variant="outline" onClick={() => void forgetDevice()}>
              Forget this device
            </Button>
          )}
        </div>

        <form
          onSubmit={(e) => void onChangePassword(e)}
          className="grid gap-2 rounded-md border p-3"
          autoComplete="off"
        >
          <p className="font-medium">Change master password</p>
          <div className="grid grid-cols-3 gap-2">
            <Input
              type="password"
              placeholder="Current"
              aria-label="Current master password"
              autoComplete="current-password"
              value={oldPw}
              onChange={(e) => {
                setOldPw(e.target.value);
              }}
            />
            <Input
              type="password"
              placeholder="New"
              aria-label="New master password"
              autoComplete="new-password"
              value={newPw}
              onChange={(e) => {
                setNewPw(e.target.value);
              }}
            />
            <Input
              type="password"
              placeholder="Repeat new"
              aria-label="Repeat new master password"
              autoComplete="new-password"
              value={newPw2}
              onChange={(e) => {
                setNewPw2(e.target.value);
              }}
            />
          </div>
          <div className="flex items-center justify-between">
            <span
              className={cn("text-xs", message?.ok ? "text-emerald-400" : "text-destructive")}
              role="status"
            >
              {message?.text}
            </span>
            <Button type="submit" size="sm" disabled={busy || !oldPw || !newPw}>
              {busy && <Loader2 className="animate-spin" />} Change
            </Button>
          </div>
        </form>
      </div>
    </VaultGate>
  );
}

function KnownHostsSection() {
  const [hosts, setHosts] = useState<KnownHost[] | null>(null);
  const askConfirm = useDialogStore((s) => s.askConfirm);

  const reload = () => {
    void knownHostsList()
      .then(setHosts)
      .catch(() => {
        setHosts([]);
      });
  };
  useEffect(reload, []);

  if (hosts === null) return null;
  if (hosts.length === 0) {
    return (
      <p className="text-muted-foreground py-6 text-center text-sm">
        No saved host keys yet. They are added when you choose “Accept &amp; save”.
      </p>
    );
  }
  return (
    <ul className="divide-y rounded-md border">
      {hosts.map((h) => (
        <li
          key={`${h.host}:${String(h.port)}:${h.keyType}`}
          className="flex items-center gap-3 px-3 py-2 text-sm"
        >
          <ServerCog className="text-muted-foreground size-4 shrink-0" />
          <div className="min-w-0 flex-1">
            <div className="truncate font-medium">
              {h.host}
              {h.port === 22 ? "" : `:${String(h.port)}`}
            </div>
            <div className="text-muted-foreground truncate font-mono text-xs">
              {h.keyType} {h.fingerprint}
            </div>
          </div>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={`Forget ${h.host}`}
            onClick={() => {
              askConfirm({
                title: "Forget host key?",
                description: `The next connection to ${h.host}:${String(h.port)} will show the fingerprint again for you to verify.`,
                confirmLabel: "Forget",
                destructive: true,
                onConfirm: async () => {
                  await knownHostsRemove(h.host, h.port);
                  reload();
                },
              });
            }}
          >
            <Trash2 />
          </Button>
        </li>
      ))}
    </ul>
  );
}

/** Folder → "Default credential…". */
export function FolderCredentialDialog() {
  const target = useDialogStore((s) => s.folderCredential);
  const close = useDialogStore((s) => s.closeFolderCredential);
  const folders = useLibraryStore((s) => s.folders);
  const credentials = useVaultStore((s) => s.credentials);
  const folder = folders.find((f) => f.id === target?.folderId);

  return (
    <Dialog
      open={target !== null}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent className="sm:max-w-sm">
        {folder && (
          <FolderCredentialForm
            key={folder.id}
            folderId={folder.id}
            folderName={folder.name}
            current={folder.defaultCredentialId}
            credentials={credentials}
            onDone={close}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

function FolderCredentialForm({
  folderId,
  folderName,
  current,
  credentials,
  onDone,
}: {
  folderId: string;
  folderName: string;
  current: string | null;
  credentials: Credential[];
  onDone: () => void;
}) {
  const [value, setValue] = useState(current ?? "");
  const [error, setError] = useState<string | null>(null);
  const openCredentials = useDialogStore((s) => s.openCredentials);

  async function save() {
    try {
      await folderSetDefaultCredential(folderId, value || null);
      await useLibraryStore.getState().load();
      await useVaultStore.getState().loadCredentials();
      onDone();
    } catch (err) {
      setError(errorText(err, "Could not save."));
    }
  }

  return (
    <div className="grid gap-4">
      <DialogHeader>
        <DialogTitle>Default credential</DialogTitle>
        <DialogDescription>
          Sessions in “{folderName}” and its subfolders that have no credential of their own use
          this one.
        </DialogDescription>
      </DialogHeader>
      <select
        className={cn(selectClass, "w-full")}
        value={value}
        aria-label="Default credential"
        onChange={(e) => {
          setValue(e.target.value);
        }}
      >
        <option value="">(none)</option>
        {credentials.map((c) => (
          <option key={c.id} value={c.id}>
            {c.name} ({c.kind === "key" ? "key" : "password"}
            {c.username ? `, ${c.username}` : ""})
          </option>
        ))}
      </select>
      {credentials.length === 0 && (
        <button
          type="button"
          className="text-brand text-left text-sm hover:underline"
          onClick={openCredentials}
        >
          No credentials yet: add one…
        </button>
      )}
      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}
      <DialogFooter>
        <Button variant="outline" onClick={onDone}>
          Cancel
        </Button>
        <Button onClick={() => void save()}>Save</Button>
      </DialogFooter>
    </div>
  );
}
