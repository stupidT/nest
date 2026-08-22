import type {
  ChatBackend,
  ChatMode,
  ClaudeConnectionStatus,
  ModelSelection,
} from "@nest/shared";

export type BackendOption = {
  id: ChatBackend;
  label: string;
  disabled: boolean;
  disabledReason: string | null;
};

export type ModelOption = {
  id: "default" | string;
  label: string;
};

export type SelectionCapsules = {
  backends: BackendOption[];
  models: ModelOption[];
  canChangeBackend: boolean;
};

export const NEST_LABEL = "Nest Agent";
export const CLAUDE_LABEL = "Claude";

export function deriveCapsules(params: {
  activeBackendId: ChatBackend | "nest" | "claude";
  boundBackend: ChatBackend | null;
  claudeEnabled: boolean;
  claudeStatus: ClaudeConnectionStatus | null;
  claudeModelIds: string[];
  claudeDefaultModelLabel: string | null;
  nestModelLabel: string | null;
}): SelectionCapsules {
  const {
    activeBackendId,
    boundBackend,
    claudeEnabled,
    claudeStatus,
    claudeModelIds,
    claudeDefaultModelLabel,
    nestModelLabel,
  } = params;

  const claudeUsable =
    claudeEnabled && (claudeStatus === "connected" || claudeStatus === "last_connected");

  const backends: BackendOption[] = [
    { id: "nest", label: NEST_LABEL, disabled: false, disabledReason: null },
  ];
  if (claudeEnabled) {
    backends.push({
      id: "claude",
      label: CLAUDE_LABEL,
      disabled: !claudeUsable,
      disabledReason: claudeUsable
        ? null
        : "Claude connection is unavailable. Test it in Settings.",
    });
  }

  const models: ModelOption[] =
    activeBackendId === "claude"
      ? [
          {
            id: "default",
            label: `${claudeDefaultModelLabel ?? "CLI Default"} (default)`,
          },
          ...claudeModelIds.map((id) => ({ id, label: id })),
        ]
      : [{ id: "default", label: nestModelLabel ?? "Default (API)" }];

  return {
    backends,
    models,
    canChangeBackend: boundBackend === null,
  };
}

export function modelSelectionFromCapsule(
  capsuleId: string,
): ModelSelection {
  if (capsuleId === "default") {
    return { kind: "default", value: null };
  }
  return { kind: "explicit", value: capsuleId };
}

export function capsuleFromModelSelection(
  selection: ModelSelection,
): string {
  return selection.kind === "default" ? "default" : (selection.value ?? "default");
}

export function capsuleModeLabel(mode: ChatMode): string {
  return mode === "agent" ? "Agent" : "Ask";
}
