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
  it("names the disabled agent with an inline settings link", () => {
    const notice = backendBlockNotice(
      descriptor({ enabled: false, reason_code: "disabled" }),
    );
    expect(notice.message).toBe("Claude is disabled for this chat.");
    expect(notice.linkText).toBe("Enable it in Settings");
    expect(notice.settingsTarget).toBe("claude-agent");
  });

  it("explains an unverified connection with a reconnect hint", () => {
    const notice = backendBlockNotice(
      descriptor({ reason_code: "connection_unverified" }),
    );
    expect(notice.message).toBe("Claude is not connected yet.");
    expect(notice.linkText).toBe("Run Test connection in Settings");
    expect(notice.settingsTarget).toBe("claude-agent");
  });

  it("explains a missing CLI path", () => {
    const notice = backendBlockNotice(descriptor({ reason_code: "cli_missing" }));
    expect(notice.message).toBe("Claude has no CLI path configured.");
    expect(notice.linkText).toBe("Set the CLI path in Settings");
  });

  it("explains a reindex-required Nest backend without a settings link", () => {
    const notice = backendBlockNotice(
      descriptor({
        id: "nest",
        label: "Nest Agent",
        reason_code: "reindex_required",
        settings_target: "general",
      }),
    );
    expect(notice.message).toBe(
      "Nest Agent is unavailable until the workspace is reindexed.",
    );
    expect(notice.linkText).toBeNull();
    expect(notice.settingsTarget).toBeNull();
  });

  it("falls back to the descriptor message with the agent label", () => {
    const notice = backendBlockNotice(
      descriptor({ reason_code: "other", message: "boom" }),
    );
    expect(notice.message).toBe("Claude: boom");
    expect(notice.linkText).toBe("Check Settings");
  });
});
