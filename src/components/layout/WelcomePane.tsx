import { KeyRound, Terminal, Zap } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useSessionStore } from "@/stores/session";

interface WelcomePaneProps {
  onQuickConnect: () => void;
}

/** Content of the Home tab. */
export function WelcomePane({ onQuickConnect }: WelcomePaneProps) {
  const hasSession = useSessionStore((s) => s.session !== null);

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
          <Button onClick={onQuickConnect} disabled={hasSession}>
            <Zap /> Quick connect
          </Button>
          <Button variant="outline" disabled>
            <KeyRound /> Set up vault
          </Button>
        </div>
        <p className="text-muted-foreground/70 text-xs">
          {hasSession
            ? "Close the open session to start another (multiple tabs arrive in Phase 2)."
            : "Saved sessions arrive in Phase 3, the vault in Phase 4."}
        </p>
      </div>
    </div>
  );
}
