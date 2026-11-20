import {
  FileInput,
  FolderPlus,
  FolderTree,
  KeyRound,
  Plus,
  Search,
  Settings,
  X,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { LibraryTree } from "@/components/sidebar/LibraryTree";
import { useDialogStore } from "@/stores/dialogs";
import { useImportDialog } from "@/stores/importDialog";
import { useLibraryStore } from "@/stores/library";
import { useUiStore } from "@/stores/ui";

/** Left panel: activity rail + saved-session tree. */
export function Sidebar() {
  const collapsed = useUiStore((s) => s.sidebarCollapsed);
  const toggleSidebar = useUiStore((s) => s.toggleSidebar);
  const query = useLibraryStore((s) => s.query);
  const setQuery = useLibraryStore((s) => s.setQuery);
  const libraryError = useLibraryStore((s) => s.error);
  const openEditor = useDialogStore((s) => s.openEditor);
  const openCredentials = useDialogStore((s) => s.openCredentials);
  const newFolder = useLibraryStore((s) => s.newFolder);
  const openImport = useImportDialog((s) => s.show);

  return (
    <aside className="bg-sidebar text-sidebar-foreground border-sidebar-border flex h-full shrink-0 border-r">
      {/* Activity rail */}
      <nav className="flex w-12 flex-col items-center gap-1 py-2" aria-label="Primary">
        <RailButton label="Sessions" active={!collapsed} onClick={toggleSidebar}>
          <FolderTree />
        </RailButton>
        <RailButton label="Vault & credentials" onClick={openCredentials}>
          <KeyRound />
        </RailButton>
        <div className="flex-1" />
        <RailButton label="Settings (coming in 1.0)" disabled>
          <Settings />
        </RailButton>
      </nav>

      {/* Session panel */}
      {!collapsed && (
        <section
          className="border-sidebar-border flex w-72 flex-col border-l"
          aria-label="Sessions"
        >
          <header className="flex h-10 items-center justify-between pr-1.5 pl-3">
            <span className="text-muted-foreground text-xs font-semibold tracking-wider uppercase">
              Sessions
            </span>
            <div className="flex items-center">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label="Import sessions"
                    disabled={libraryError !== null}
                    onClick={openImport}
                  >
                    <FileInput />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Import from MobaXterm or PuTTY</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label="New folder"
                    disabled={libraryError !== null}
                    onClick={() => void newFolder(null)}
                  >
                    <FolderPlus />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>New folder</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label="New session"
                    disabled={libraryError !== null}
                    onClick={() => {
                      openEditor({ mode: "create", folderId: null });
                    }}
                  >
                    <Plus />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>New session</TooltipContent>
              </Tooltip>
            </div>
          </header>
          <div className="px-3 pb-2">
            <div className="bg-background/60 focus-within:border-ring flex h-8 items-center gap-2 rounded-md border px-2">
              <Search className="text-muted-foreground size-3.5 shrink-0" />
              <input
                type="text"
                value={query}
                onChange={(e) => {
                  setQuery(e.target.value);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setQuery("");
                }}
                placeholder="Search name, host, tag…"
                aria-label="Search sessions"
                spellCheck={false}
                className="placeholder:text-muted-foreground min-w-0 flex-1 bg-transparent text-sm outline-none"
              />
              {query && (
                <button
                  type="button"
                  aria-label="Clear search"
                  className="text-muted-foreground hover:text-foreground"
                  onClick={() => {
                    setQuery("");
                  }}
                >
                  <X className="size-3.5" />
                </button>
              )}
            </div>
          </div>
          <Separator />
          {libraryError ? (
            <p role="alert" className="text-destructive p-4 text-sm">
              {libraryError}
            </p>
          ) : (
            <LibraryTree />
          )}
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
