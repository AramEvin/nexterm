/**
 * Prompts Rust is waiting on (host-key decisions, 2FA codes), oldest first.
 * The dialogs show the first one; answering removes it.
 */
import { create } from "zustand";

import { sshPromptRespond } from "@/lib/ipc";
import type { PromptResponse, SshPrompt } from "@/types/ipc";

interface PromptState {
  queue: SshPrompt[];
  push: (prompt: SshPrompt) => void;
  respond: (promptId: string, response: PromptResponse) => Promise<void>;
}

export const usePromptStore = create<PromptState>()((set) => ({
  queue: [],
  push: (prompt) => {
    set((s) => ({ queue: [...s.queue, prompt] }));
  },
  respond: async (promptId, response) => {
    set((s) => ({ queue: s.queue.filter((p) => p.promptId !== promptId) }));
    // The connection may have given up already (timeout); that's fine.
    await sshPromptRespond(promptId, response).catch(() => undefined);
  },
}));
