import { Home, Plus, TerminalSquare, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { useSessionStore, type TabKey } from "@/stores/session";

interface TabBarProps {
  onNewSession: () => void;
}

/** Tab strip: Home + (Phase 1) a single session tab. */
export function TabBar({ onNewSession }: TabBarProps) {
  const session = useSessionStore((s) => s.session);
  const activeTab = useSessionStore((s) => s.activeTab);
  const setActiveTab = useSessionStore((s) => s.setActiveTab);
  const closeTab = useSessionStore((s) => s.closeTab);

  return (
    <div
      className="bg-sidebar border-sidebar-border flex h-10 shrink-0 items-end gap-1 border-b px-2"
      role="tablist"
      aria-label="Open sessions"
    >
      <Tab tab="home" active={activeTab === "home"} onSelect={setActiveTab}>
        <Home className="text-brand size-3.5" />
        Home
      </Tab>

      {session && (
        <Tab tab="session" active={activeTab === "session"} onSelect={setActiveTab}>
          <TerminalSquare
            className={cn(
              "size-3.5",
              session.state === "connected" ? "text-emerald-400" : "text-destructive",
            )}
          />
          <span className="max-w-48 truncate">
            {session.username}@{session.host}
          </span>
          <span
            role="button"
            tabIndex={0}
            aria-label="Close tab"
            className="hover:bg-accent -mr-1 ml-1 rounded p-0.5"
            onClick={(e) => {
              e.stopPropagation();
              void closeTab();
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") void closeTab();
            }}
          >
            <X className="size-3.5" />
          </span>
        </Tab>
      )}

      <Tooltip>
        <TooltipTrigger asChild>
          {/* span wrapper so the tooltip still works when the button is disabled */}
          <span className="mb-0.5">
            <Button
              variant="ghost"
              size="icon-sm"
              disabled={session !== null}
              aria-label="Quick connect"
              onClick={onNewSession}
            >
              <Plus />
            </Button>
          </span>
        </TooltipTrigger>
        <TooltipContent>
          {session ? "One session at a time until Phase 2 (tabs)" : "Quick connect"}
        </TooltipContent>
      </Tooltip>
    </div>
  );
}

interface TabProps {
  tab: TabKey;
  active: boolean;
  onSelect: (tab: TabKey) => void;
  children: React.ReactNode;
}

function Tab({ tab, active, onSelect, children }: TabProps) {
  return (
    <div
      role="tab"
      aria-selected={active}
      tabIndex={0}
      onClick={() => {
        onSelect(tab);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") onSelect(tab);
      }}
      className={cn(
        "flex h-8 cursor-default items-center gap-2 rounded-t-md border border-b-0 px-3 text-sm",
        active ? "bg-background" : "text-muted-foreground hover:text-foreground border-transparent",
      )}
    >
      {children}
    </div>
  );
}
