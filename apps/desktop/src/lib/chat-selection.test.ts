import { describe, expect, it } from "vitest";
import {
  capsuleFromModelSelection,
  deriveCapsules,
  modelSelectionFromCapsule,
} from "./chat-selection";

describe("deriveCapsules", () => {
  it("always offers Nest and hides Claude when the toggle is off", () => {
    const capsules = deriveCapsules({
      boundBackend: null,
      claudeEnabled: false,
      claudeStatus: "connected",
      claudeModelIds: ["glm-5.3"],
      nestModelLabel: "gpt-4o-mini",
    });
    expect(capsules.backends).toHaveLength(1);
    expect(capsules.backends[0].id).toBe("nest");
    expect(capsules.canChangeBackend).toBe(true);
  });

  it("offers Claude enabled when connected or last-connected", () => {
    for (const status of ["connected", "last_connected"] as const) {
      const capsules = deriveCapsules({
        boundBackend: null,
        claudeEnabled: true,
        claudeStatus: status,
        claudeModelIds: [],
        nestModelLabel: null,
      });
      expect(capsules.backends).toHaveLength(2);
      expect(capsules.backends[1].disabled).toBe(false);
    }
  });

  it("keeps a disabled Claude entry with a reason when unavailable", () => {
    const capsules = deriveCapsules({
      boundBackend: null,
      claudeEnabled: true,
      claudeStatus: "unavailable",
      claudeModelIds: [],
      nestModelLabel: null,
    });
    expect(capsules.backends).toHaveLength(2);
    expect(capsules.backends[1].disabled).toBe(true);
    expect(capsules.backends[1].disabledReason).toContain("unavailable");
  });

  it("locks the backend capsule once bound", () => {
    const capsules = deriveCapsules({
      boundBackend: "claude",
      claudeEnabled: true,
      claudeStatus: "connected",
      claudeModelIds: ["glm-5.3"],
      nestModelLabel: null,
    });
    expect(capsules.canChangeBackend).toBe(false);
    expect(capsules.models).toEqual([
      { id: "default", label: "CLI Default" },
      { id: "glm-5.3", label: "glm-5.3" },
    ]);
  });

  it("shows the Nest API model for unbound Nest selection", () => {
    const capsules = deriveCapsules({
      boundBackend: "nest",
      claudeEnabled: true,
      claudeStatus: "connected",
      claudeModelIds: ["glm-5.3"],
      nestModelLabel: "gpt-4o-mini",
    });
    expect(capsules.models).toEqual([
      { id: "default", label: "gpt-4o-mini" },
    ]);
  });

  it("falls back to Nest models when Claude is unusable and unbound", () => {
    const capsules = deriveCapsules({
      boundBackend: null,
      claudeEnabled: true,
      claudeStatus: "unavailable",
      claudeModelIds: ["glm-5.3"],
      nestModelLabel: "gpt-4o-mini",
    });
    expect(capsules.models).toEqual([
      { id: "default", label: "gpt-4o-mini" },
    ]);
  });
});

describe("model capsule mapping", () => {
  it("round-trips default and explicit selections", () => {
    const def = { kind: "default" as const, value: null };
    expect(capsuleFromModelSelection(def)).toBe("default");
    expect(modelSelectionFromCapsule("default")).toEqual(def);

    const explicit = { kind: "explicit" as const, value: "glm-5.3" };
    expect(capsuleFromModelSelection(explicit)).toBe("glm-5.3");
    expect(modelSelectionFromCapsule("glm-5.3")).toEqual(explicit);
  });
});
