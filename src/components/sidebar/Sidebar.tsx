import { FolderTree, Plus, Search, Settings } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/ui";

/** Left panel: activity rail + session tree (tree arrives in Phase 3). */
export function Sidebar() {
  const collapsed = useUiStore((s) => s.sidebarCollapsed);
  const toggleSidebar = useUiStore((s) => s.toggleSidebar);

  return (
    <aside className="bg-sidebar text-sidebar-foreground border-sidebar-border flex h-full shrink-0 border-r">
      {/* Activity rail */}
      <nav className="flex w-12 flex-col items-center gap-1 py-2" aria-label="Primary">
        <RailButton label="Sessions" active={!collapsed} onClick={toggleSidebar}>
          <FolderTree />
        </RailButton>
        <div className="flex-1" />
        <RailButton label="Settings (Phase 7)" disabled>
          <Settings />
        </RailButton>
      </nav>

      {/* Session panel */}
      {!collapsed && (
        <section
          className="border-sidebar-border flex w-64 flex-col border-l"
          aria-label="Sessions"
        >
          <header className="flex h-10 items-center justify-between px-3">
            <span className="text-muted-foreground text-xs font-semibold tracking-wider uppercase">
              Sessions
            </span>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button variant="ghost" size="icon-sm" disabled aria-label="New session">
                  <Plus />
                </Button>
              </TooltipTrigger>
              <TooltipContent>New session (Phase 3)</TooltipContent>
            </Tooltip>
          </header>
          <div className="px-3 pb-2">
            <div className="bg-background/60 text-muted-foreground flex h-8 items-center gap-2 rounded-md border px-2 text-sm">
              <Search className="size-3.5" />
              <span>Search sessions…</span>
            </div>
          </div>
          <Separator />
          <div className="text-muted-foreground flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-sm">
            <FolderTree className="size-8 opacity-40" />
            <p>No saved sessions yet.</p>
          </div>
        </section>
      )}
    </aside>
  );
}

interface RailButtonProps {
  label: string;
  active?: boolean;
  disabled?: boolean;
  onClick?: () => void;
  children: React.ReactNode;
}

function RailButton({ label, active, disabled, onClick, children }: RailButtonProps) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          aria-label={label}
          aria-pressed={active}
          disabled={disabled}
          onClick={onClick}
          className={cn(
            "text-muted-foreground relative",
            active &&
              "text-sidebar-foreground before:bg-brand before:absolute before:top-2 before:bottom-2 before:-left-0.5 before:w-0.5 before:rounded-full",
          )}
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}
