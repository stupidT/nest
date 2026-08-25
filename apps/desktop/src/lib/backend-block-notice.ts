import type { BackendDescriptor } from "@nest/shared";

export type BackendBlockNotice = {
  message: string;
  linkText: string | null;
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
      return `${label} is disabled for this chat.`;
    }
    switch (reason) {
      case "cli_missing":
        return `${label} has no CLI path configured.`;
      case "connection_unverified":
        return `${label} is not connected yet.`;
      case "reindex_required":
        return `${label} is unavailable until the workspace is reindexed.`;
      case "unknown_backend":
        return `${label} is not installed in this build. Start a new chat with an available agent.`;
      default:
        return descriptor.message
          ? `${label}: ${descriptor.message}`
          : `${label} is unavailable.`;
    }
  })();

  const linkText = (() => {
    if (!descriptor.enabled) {
      return "Enable it in Settings";
    }
    switch (reason) {
      case "cli_missing":
        return "Set the CLI path in Settings";
      case "connection_unverified":
        return "Run Test connection in Settings";
      case "reindex_required":
        return null;
      case "unknown_backend":
        return null;
      default:
        return target ? "Check Settings" : null;
    }
  })();

  return {
    message,
    linkText,
    settingsTarget: linkText ? target : null,
  };
}

export function backendBlockNoticeFromReason(
  reason: string,
): BackendBlockNotice | null {
  return {
    message: reason,
    linkText: null,
    settingsTarget: null,
  };
}
