import { useEffect } from "react";

import { ConfirmDialog } from "@/components/dialogs/ConfirmDialog";
import { ConnectDialog } from "@/components/dialogs/ConnectDialog";
import { SessionEditorDialog } from "@/components/dialogs/SessionEditorDialog";
import { StatusBar } from "@/components/layout/StatusBar";
import { TabBar } from "@/components/layout/TabBar";
import { WelcomePane } from "@/components/layout/WelcomePane";
import { Sidebar } from "@/components/sidebar/Sidebar";
import { TerminalView } from "@/components/terminal/TerminalView";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useDialogStore } from "@/stores/dialogs";
import { useLibraryStore } from "@/stores/library";
import { HOME_TAB, useSessionStore } from "@/stores/session";
import { useUiStore } from "@/stores/ui";

/** Application shell: sidebar | (tabs + content) / status bar. */
export default function App() {
  const loadAppInfo = useUiStore((s) => s.loadAppInfo);
  const tabs = useSessionStore((s) => s.tabs);
  const activeTabId = useSessionStore((s) => s.activeTabId);
  const loadLibrary = useLibraryStore((s) => s.load);
  const openConnect = useDialogStore((s) => s.openConnect);

  useEffect(() => {
    void loadAppInfo();
    void loadLibrary();
  }, [loadAppInfo, loadLibrary]);

  const openQuickConnect = () => {
    openConnect({ mode: "quick" });
  };

  return (
    <TooltipProvider>
      <div className="flex h-full flex-col">
        <div className="flex min-h-0 flex-1">
          <Sidebar />
          <main className="flex min-w-0 flex-1 flex-col">
            <TabBar onNewSession={openQuickConnect} />
            <div className="bg-background min-h-0 flex-1">
              {activeTabId === HOME_TAB && <WelcomePane onQuickConnect={openQuickConnect} />}
              {tabs.map((tab) => (
                <TerminalView key={tab.id} tab={tab} visible={activeTabId === tab.id} />
              ))}
            </div>
          </main>
        </div>
        <StatusBar />
      </div>
      <ConnectDialog />
      <SessionEditorDialog />
      <ConfirmDialog />
    </TooltipProvider>
  );
}
