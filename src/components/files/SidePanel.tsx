import { useEffect } from "react";
import { X } from "lucide-react";

import { FilePane } from "@/components/files/FilePane";
import { TransferQueue } from "@/components/files/TransferQueue";
import { useExplorerDrop } from "@/components/files/useFileDrop";
import { Button } from "@/components/ui/button";
import { openFilesForTerminal, useFilesStore } from "@/stores/files";
import { tabTitle, type SessionTab } from "@/stores/session";

/**
 * SFTP panel next to a terminal (MobaXterm-style). Opens its own SFTP session
 * with the terminal's remembered login, so no password is asked again.
 */
export function SidePanel({
  terminal,
  visible,
  onClose,
}: {
  terminal: SessionTab;
  visible: boolean;
  onClose: () => void;
}) {
  const tab = useFilesStore((s) => s.tabs.find((t) => t.side === terminal.id));
  const openSide = useFilesStore((s) => s.openSide);
  const target = useExplorerDrop(tab?.id ?? "", visible && tab !== undefined);

  useEffect(() => {
    if (tab || !terminal.connectionId) return;
    openSide(terminal.id, {
      kind: "connection",
      connectionId: terminal.connectionId,
      name: tabTitle(terminal),
      host: terminal.host,
      port: terminal.port,
    });
  }, [tab, terminal, openSide]);

  return (
    <aside
      aria-label="SFTP panel"
      className="bg-background flex w-[400px] max-w-[45%] min-w-[280px] shrink-0 flex-col border-l"
    >
      <div className="text-muted-foreground flex h-7 shrink-0 items-center justify-between border-b px-2 text-xs">
        <span>Files (SFTP)</span>
        <Button variant="ghost" size="icon-sm" aria-label="Close SFTP panel" onClick={onClose}>
          <X />
        </Button>
      </div>
      {tab ? (
        <div className="flex min-h-0 flex-1 flex-col">
          <FilePane
            tabId={tab.id}
            index={0}
            pane={tab.panes[0]}
            active
            mode="side"
            onOpenFull={() => {
              openFilesForTerminal(terminal);
            }}
            dropDir={target?.pane === 0 ? target.dir : null}
            onDragStart={() => undefined}
          />
          <TransferQueue />
        </div>
      ) : (
        <p className="text-muted-foreground p-4 text-sm">Connect the terminal first.</p>
      )}
    </aside>
  );
}
