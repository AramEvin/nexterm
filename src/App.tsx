import { useEffect } from "react";

import { StatusBar } from "@/components/layout/StatusBar";
import { TabBar } from "@/components/layout/TabBar";
import { WelcomePane } from "@/components/layout/WelcomePane";
import { Sidebar } from "@/components/sidebar/Sidebar";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useUiStore } from "@/stores/ui";

/** Application shell: sidebar | (tabs + content) / status bar. */
export default function App() {
  const loadAppInfo = useUiStore((s) => s.loadAppInfo);

  useEffect(() => {
    void loadAppInfo();
  }, [loadAppInfo]);

  return (
    <TooltipProvider>
      <div className="flex h-full flex-col">
        <div className="flex min-h-0 flex-1">
          <Sidebar />
          <main className="flex min-w-0 flex-1 flex-col">
            <TabBar />
            <div className="bg-background min-h-0 flex-1">
              <WelcomePane />
            </div>
          </main>
        </div>
        <StatusBar />
      </div>
    </TooltipProvider>
  );
}
