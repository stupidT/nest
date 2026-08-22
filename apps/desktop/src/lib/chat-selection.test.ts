import { describe, expect, it } from "vitest";
import {
  capsuleFromModelSelection,
  deriveCapsules,
  modelSelectionFromCapsule,
} from "./chat-selection";

describe("deriveCapsules", () => {
  it("always offers Nest and hides Claude when the toggle is off", () => {
    const capsules = deriveCapsules({
      activeBackendId: "nest",
      boundBackend: null,
      claudeEnabled: false,
      claudeStatus: "connected",
      claudeModelIds: ["glm-5.3"],
      claudeDefaultModelLabel: null,
      nestModelLabel: "gpt-4o-mini",
    });
    expect(capsules.backends).toHaveLength(1);
    expect(capsules.backends[0].id).toBe("nest");
    expect(capsules.canChangeBackend).toBe(true);
  });

  it("offers Claude enabled when connected or last-connected", () => {
    for (const status of ["connected", "last_connected"] as const) {
      const capsules = deriveCapsules({
        activeBackendId: "nest",
        boundBackend: null,
        claudeEnabled: true,
        claudeStatus: status,
        claudeModelIds: [],
        claudeDefaultModelLabel: null,
        nestModelLabel: null,
      });
      expect(capsules.backends).toHaveLength(2);
      expect(capsules.backends[1].disabled).toBe(false);
    }
  });

  it("keeps a disabled Claude entry with a reason when unavailable", () => {
    const capsules = deriveCapsules({
      activeBackendId: "nest",
      boundBackend: null,
      claudeEnabled: true,
      claudeStatus: "unavailable",
      claudeModelIds: [],
      claudeDefaultModelLabel: null,
      nestModelLabel: null,
    });
    expect(capsules.backends).toHaveLength(2);
    expect(capsules.backends[1].disabled).toBe(true);
    expect(capsules.backends[1].disabledReason).toContain("unavailable");
  });

  it("shows Nest models only when the active selection is Nest", () => {
    const capsules = deriveCapsules({
      activeBackendId: "nest",
      boundBackend: null,
      claudeEnabled: true,
      claudeStatus: "connected",
      claudeModelIds: ["glm-5.3"],
      claudeDefaultModelLabel: "glm-5.3[1m]",
      nestModelLabel: "gpt-4o-mini",
    });
    expect(capsules.models).toEqual([
      { id: "default", label: "gpt-4o-mini" },
    ]);
  });

  it("shows Claude models when the active selection is Claude", () => {
    const capsules = deriveCapsules({
      activeBackendId: "claude",
      boundBackend: null,
      claudeEnabled: true,
      claudeStatus: "connected",
      claudeModelIds: ["glm-5.3"],
      claudeDefaultModelLabel: "glm-5.3[1m]",
      nestModelLabel: "gpt-4o-mini",
    });
    expect(capsules.models).toEqual([
      { id: "default", label: "glm-5.3[1m]" },
      { id: "glm-5.3", label: "glm-5.3" },
    ]);
  });

  it("drops the explicit entry duplicating the default model label", () => {
    const capsules = deriveCapsules({
      activeBackendId: "claude",
      boundBackend: null,
      claudeEnabled: true,
      claudeStatus: "connected",
      claudeModelIds: ["glm-5.3[1m]", "glm-5.3"],
      claudeDefaultModelLabel: "glm-5.3[1m]",
      nestModelLabel: null,
    });
    expect(capsules.models).toEqual([
      { id: "default", label: "glm-5.3[1m]" },
      { id: "glm-5.3", label: "glm-5.3" },
    ]);
  });

  it("falls back to a CLI Default label when no observed model exists", () => {
    const capsules = deriveCapsules({
      activeBackendId: "claude",
      boundBackend: null,
      claudeEnabled: true,
      claudeStatus: "connected",
      claudeModelIds: [],
      claudeDefaultModelLabel: null,
      nestModelLabel: null,
    });
    expect(capsules.models).toEqual([{ id: "default", label: "CLI Default" }]);
  });

  it("locks the backend capsule once bound", () => {
    const capsules = deriveCapsules({
      activeBackendId: "claude",
      boundBackend: "claude",
      claudeEnabled: true,
      claudeStatus: "connected",
      claudeModelIds: ["glm-5.3"],
      claudeDefaultModelLabel: null,
      nestModelLabel: null,
    });
    expect(capsules.canChangeBackend).toBe(false);
    expect(capsules.models).toEqual([
      { id: "default", label: "CLI Default" },
      { id: "glm-5.3", label: "glm-5.3" },
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
