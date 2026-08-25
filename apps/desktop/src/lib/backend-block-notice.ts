import type { BackendDescriptor } from "@nest/shared";

export type BackendBlockNotice = {
  message: string;
  reasonCode: string | null;
  settingsTarget: "claude-agent" | "general" | null;
};

const CLAUDE_LABEL = "Claude";

export function backendBlockNotice(
  descriptor: BackendDescriptor,
): BackendBlockNotice {
  const label = descriptor.id === "claude" ? CLAUDE_LABEL : descriptor.label;
  const reason = descriptor.reason_code ?? "";
  const target: "claude-agent" | "general" | null =
    descriptor.settings_target === "claude_agent"
      ? "claude-agent"
      : descriptor.settings_target === "general"
        ? "general"
        : null;

  const message = (() => {
    if (!descriptor.enabled) {
      return `${label} is disabled. Enable it in Settings to use this chat.`;
    }
    switch (reason) {
      case "cli_missing":
        return `${label} has no CLI path configured. Set it in Settings to reconnect.`;
      case "connection_unverified":
        return `${label} is not connected yet. Run Test connection in Settings to reconnect.`;
      case "reindex_required":
        return `${label} is unavailable until the workspace is reindexed. Trigger a reindex from Library.`;
      case "unknown_backend":
        return `${label} is not installed in this build. Start a new chat with an available agent.`;
      default:
        return descriptor.message
          ? `${label}: ${descriptor.message}`
          : `${label} is unavailable. Check Settings to reconnect.`;
    }
  })();

  return {
    message,
    reasonCode: reason || null,
    settingsTarget: target,
  };
}

export function backendBlockNoticeFromReason(  reason: string,
): BackendBlockNotice | null {
  return {
    message: reason,
    reasonCode: null,
    settingsTarget: null,
  };
}
