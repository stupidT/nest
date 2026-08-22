import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { AppSettings, ClaudeConnectionReport } from "@nest/shared";
import { CheckCircle2, LoaderCircle, Sparkles, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import { queryKeys } from "@/lib/query-keys";
import { GeneralGroup } from "./GeneralGroup";

type ClaudeDraft = {
  enabled: boolean;
  cliPath: string;
  customModels: string;
};

function useClaudeAgentSettings(settingsQuery: {
  data: AppSettings | undefined;
}) {
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState<ClaudeDraft>({
    enabled: false,
    cliPath: "",
    customModels: "",
  });
  const [hydrated, setHydrated] = useState(false);
  const [testResult, setTestResult] = useState<ClaudeConnectionReport | null>(
    null,
  );
  const [stale, setStale] = useState(false);

  useEffect(() => {
    if (!settingsQuery.data || hydrated) return;
    setDraft({
      enabled: settingsQuery.data.claude_agent_enabled,
      cliPath: settingsQuery.data.claude_cli_path,
      customModels: settingsQuery.data.claude_custom_models,
    });
    setHydrated(true);
  }, [settingsQuery.data, hydrated]);

  const connectionQuery = useQuery({
    queryKey: queryKeys.claudeConnection,
    queryFn: api.claudeConnectionStatus,
  });

  const dirty =
    hydrated &&
    (draft.enabled !== (settingsQuery.data?.claude_agent_enabled ?? false) ||
      draft.cliPath !== (settingsQuery.data?.claude_cli_path ?? "") ||
      draft.customModels !==
        (settingsQuery.data?.claude_custom_models ?? ""));

  const markDirty = () => setStale(true);

  const detect = useMutation({
    mutationFn: () => api.claudeDetectCli(draft.cliPath.trim() || undefined),
    onSuccess: (detection) => {
      setDraft((prev) => ({ ...prev, cliPath: detection.resolved_path }));
      markDirty();
    },
    onError: (e: unknown) => {
      toast.error(t("settings.claude.couldNotDetect"), {
        description: e instanceof Error ? e.message : String(e),
      });
    },
  });

  const test = useMutation({
    mutationFn: () => api.claudeTestConnection(draft.cliPath),
    onSuccess: (report) => setTestResult(report),
    onError: (e: unknown) => {
      toast.error(t("settings.claude.couldNotTest"), {
        description: e instanceof Error ? e.message : String(e),
      });
    },
  });

  const save = useMutation({
    mutationFn: () =>
      api.claudeSaveSettings({
        enabled: draft.enabled,
        cliPath: draft.cliPath,
        customModels: draft.customModels,
      }),
    onSuccess: (report) => {
      setTestResult(null);
      setStale(false);
      void queryClient.invalidateQueries({ queryKey: queryKeys.settings });
      void queryClient.invalidateQueries({
        queryKey: queryKeys.claudeConnection,
      });
      void queryClient.invalidateQueries({ queryKey: queryKeys.chatSessions });
      if (report.status === "connected") {
        toast.success(t("settings.claude.statusConnected"));
      } else if (report.message) {
        toast.error(t("settings.claude.statusDisconnected"), {
          description: report.message,
        });
      }
    },
    onError: (e: unknown) => {
      toast.error(t("settings.claude.couldNotSave"), {
        description: e instanceof Error ? e.message : String(e),
      });
    },
  });

  const persistedStatus =
    stale || testResult
      ? null
      : connectionQuery.data &&
          connectionQuery.data.configured_cli_path === draft.cliPath.trim()
        ? connectionQuery.data
        : null;

  return {
    draft,
    setDraft,
    detect,
    test,
    save,
    dirty,
    markDirty,
    testResult,
    persistedStatus,
  };
}

export function ClaudeAgentSettingsSection({
  settingsQuery,
}: {
  settingsQuery: { data: AppSettings | undefined };
}) {
  const { t } = useI18n();
  const {
    draft,
    setDraft,
    detect,
    test,
    save,
    dirty,
    markDirty,
    testResult,
    persistedStatus,
  } = useClaudeAgentSettings(settingsQuery);

  const displayReport = testResult ?? persistedStatus;
  const reportConnected =
    displayReport?.status === "connected" ||
    displayReport?.status === "last_connected";

  return (
    <GeneralGroup
      icon={Sparkles}
      title={t("settings.claude.group")}
      help={<p>{t("settings.claude.groupDescription")}</p>}
    >
      <div className="flex items-start justify-between gap-4 rounded-lg bg-muted/40 px-3 py-3">
        <div className="min-w-0 space-y-1">
          <Label htmlFor="claude-enabled" className="text-sm font-medium">
            {t("settings.claude.enabled")}
          </Label>
          <p className="text-xs text-muted-foreground">
            {t("settings.claude.enabledDescription")}
          </p>
        </div>
        <Switch
          id="claude-enabled"
          checked={draft.enabled}
          onCheckedChange={(checked) => {
            setDraft((prev) => ({ ...prev, enabled: checked }));
            markDirty();
          }}
          aria-label={t("settings.claude.enabled")}
        />
      </div>
      <Field
        label={t("settings.claude.cliPath")}
        description={t("settings.claude.cliPathDescription")}
      >
        <div className="flex min-w-0 gap-2">
          <Input
            value={draft.cliPath}
            onChange={(e) => {
              setDraft((prev) => ({ ...prev, cliPath: e.target.value }));
              markDirty();
            }}
            placeholder="claude.exe · cli-wrapper.cjs · empty = auto-detect"
            className="min-w-0 flex-1 font-mono text-xs"
          />
          <Button
            type="button"
            variant="outline"
            className="shrink-0"
            disabled={detect.isPending}
            onClick={() => detect.mutate()}
          >
            {detect.isPending && (
              <LoaderCircle className="size-4 animate-spin" />
            )}
            {detect.isPending
              ? t("settings.claude.detecting")
              : t("settings.claude.autoDetect")}
          </Button>
        </div>
      </Field>
      <Field
        label={t("settings.claude.testConnection")}
        description={
          draft.enabled
            ? t("settings.claude.saveAndConnect")
            : t("settings.claude.save")
        }
      >
        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={test.isPending}
            onClick={() => test.mutate()}
          >
            {test.isPending && (
              <LoaderCircle className="size-3.5 animate-spin" />
            )}
            {test.isPending
              ? t("settings.testing")
              : t("settings.claude.testConnection")}
          </Button>
          <Button
            type="button"
            size="sm"
            disabled={save.isPending}
            onClick={() => save.mutate()}
          >
            {save.isPending && (
              <LoaderCircle className="size-3.5 animate-spin" />
            )}
            {save.isPending
              ? t("settings.claude.saving")
              : draft.enabled
                ? t("settings.claude.saveAndConnect")
                : t("settings.claude.save")}
          </Button>
        </div>
        {dirty && (
          <p className="text-xs text-muted-foreground">
            {t("settings.claude.notSaved")}
          </p>
        )}
        {displayReport && (
          <div className="space-y-1 rounded-md border bg-muted/30 px-3 py-2">
            <p
              className={
                reportConnected
                  ? "flex items-center gap-1.5 text-xs text-primary"
                  : "flex items-center gap-1.5 text-xs text-destructive"
              }
            >
              {reportConnected ? (
                <CheckCircle2 className="size-3.5 shrink-0" />
              ) : (
                <XCircle className="size-3.5 shrink-0" />
              )}
              {reportConnected
                ? displayReport.status === "last_connected"
                  ? t("settings.claude.statusLastConnected")
                  : t("settings.claude.statusConnected")
                : (displayReport.message ??
                  t("settings.claude.statusDisconnected"))}
            </p>
            {reportConnected && (
              <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs text-muted-foreground">
                <dt>{t("settings.claude.resolvedPath")}</dt>
                <dd className="truncate font-mono">
                  {displayReport.resolved_cli_path}
                </dd>
                <dt>{t("settings.claude.cliVersion")}</dt>
                <dd className="font-mono">{displayReport.cli_version}</dd>
                <dt>{t("settings.claude.effectiveModel")}</dt>
                <dd className="font-mono">
                  {displayReport.effective_model}
                </dd>
                <dt>{t("settings.claude.testedAt")}</dt>
                <dd className="font-mono">{displayReport.tested_at}</dd>
              </dl>
            )}
          </div>
        )}
      </Field>
      <Field
        label={t("settings.claude.customModels")}
        description={t("settings.claude.customModelsDescription")}
      >
        <Textarea
          rows={4}
          value={draft.customModels}
          onChange={(e) => {
            setDraft((prev) => ({ ...prev, customModels: e.target.value }));
            markDirty();
          }}
          placeholder={"glm-5.3\nclaude-sonnet-4-5"}
          className="font-mono text-xs"
        />
      </Field>
    </GeneralGroup>
  );
}
