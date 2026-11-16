import { FolderOpen, KeyRound, Terminal, Zap } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useDialogStore } from "@/stores/dialogs";
import { useFilesStore } from "@/stores/files";
import { useVaultStore } from "@/stores/vault";

interface WelcomePaneProps {
  onQuickConnect: () => void;
}

/** Content of the Home tab. */
export function WelcomePane({ onQuickConnect }: WelcomePaneProps) {
  const initialized = useVaultStore((s) => s.status?.initialized ?? false);
  const openVaultSetup = useDialogStore((s) => s.openVaultSetup);
  const openCredentials = useDialogStore((s) => s.openCredentials);
  const openFiles = useFilesStore((s) => s.openTab);
  return (
    <div className="flex h-full items-center justify-center p-8">
      <div className="flex max-w-md flex-col items-center gap-6 text-center">
        <div className="bg-brand/15 text-brand flex size-16 items-center justify-center rounded-2xl">
          <Terminal className="size-8" />
        </div>
        <div className="space-y-2">
          <h1 className="text-2xl font-semibold tracking-tight">Welcome to NexTerm</h1>
          <p className="text-muted-foreground text-sm">
            A fast, open-source SSH &amp; SFTP client with an encrypted credential vault.
          </p>
        </div>
        <div className="flex gap-2">
          <Button onClick={onQuickConnect}>
            <Zap /> Quick connect
          </Button>
          <Button
            variant="outline"
            onClick={() => {
              openFiles();
            }}
          >
            <FolderOpen /> File manager
          </Button>
          <Button variant="outline" onClick={initialized ? openCredentials : openVaultSetup}>
            <KeyRound /> {initialized ? "Credentials" : "Set up vault"}
          </Button>
        </div>
        <p className="text-muted-foreground/70 text-xs">
          Tip: double-click a saved session to connect. Link a saved password or key in the
          session&apos;s Login settings to skip typing it. Right-click a session → Open in file
          manager to browse its files.
        </p>
      </div>
    </div>
  );
}
