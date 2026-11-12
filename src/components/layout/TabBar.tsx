import { Home, Plus } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

/** Tab strip. Only the static "Home" tab exists until Phase 1/2. */
export function TabBar() {
  return (
    <div
      className="bg-sidebar border-sidebar-border flex h-10 shrink-0 items-end gap-1 border-b px-2"
      role="tablist"
      aria-label="Open sessions"
    >
      <div
        role="tab"
        aria-selected="true"
        className="bg-background flex h-8 items-center gap-2 rounded-t-md border border-b-0 px-3 text-sm"
      >
        <Home className="text-brand size-3.5" />
        Home
      </div>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button variant="ghost" size="icon-sm" className="mb-0.5" disabled aria-label="New tab">
            <Plus />
          </Button>
        </TooltipTrigger>
        <TooltipContent>New tab (Phase 1)</TooltipContent>
      </Tooltip>
    </div>
  );
}
