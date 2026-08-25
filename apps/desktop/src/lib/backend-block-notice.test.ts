import type { BackendDescriptor } from "@nest/shared";
import { describe, expect, it } from "vitest";
import { backendBlockNotice } from "./backend-block-notice";

function descriptor(
  options: Partial<BackendDescriptor>,
): BackendDescriptor {
  return {
    id: "claude",
    label: "Claude",
    enabled: true,
    availability: "unavailable",
    reason_code: null,
    message: null,
    modes: [],
    models: [],
    native_tool_profile: "test",
    knowledge_profile: "test",
    settings_target: "claude_agent",
    ...options,
  };
}

describe("backendBlockNotice", () => {
  it("names the disabled agent and links to its settings section", () => {
    const notice = backendBlockNotice(
      descriptor({ enabled: false, reason_code: "disabled" }),
    );
    expect(notice.message).toBe(
      "Claude is disabled. Enable it in Settings to use this chat.",
    );
    expect(notice.reasonCode).toBe("disabled");
    expect(notice.settingsTarget).toBe("claude-agent");
  });

  it("explains an unverified connection and offers the settings target", () => {
    const notice = backendBlockNotice(
      descriptor({ reason_code: "connection_unverified" }),
    );
    expect(notice.message).toBe(
      "Claude is not connected yet. Run Test connection in Settings to reconnect.",
    );
    expect(notice.settingsTarget).toBe("claude-agent");
  });

  it("explains a missing CLI path", () => {
    const notice = backendBlockNotice(descriptor({ reason_code: "cli_missing" }));
    expect(notice.message).toBe(
      "Claude has no CLI path configured. Set it in Settings to reconnect.",
    );
  });

  it("explains a reindex-required Nest backend", () => {
    const notice = backendBlockNotice(
      descriptor({
        id: "nest",
        label: "Nest Agent",
        reason_code: "reindex_required",
        settings_target: "general",
      }),
    );
    expect(notice.message).toBe(
      "Nest Agent is unavailable until the workspace is reindexed. Trigger a reindex from Library.",
    );
    expect(notice.settingsTarget).toBe("general");
  });

  it("falls back to the descriptor message with the agent label", () => {
    const notice = backendBlockNotice(
      descriptor({ reason_code: "other", message: "boom" }),
    );
    expect(notice.message).toBe("Claude: boom");
  });
});
