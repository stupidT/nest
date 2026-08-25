import { CheckCircle2, LoaderCircle, Plus, X, XCircle } from "lucide-react";
import { useEffect, useRef } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useI18n } from "@/lib/i18n";
import { isDuplicateRow } from "./model-rows";

export type ModelRowStatus = "idle" | "testing" | "ok" | "fail";
export type ModelRowFailure = { message: string | null };
export type ModelRowStatuses = Record<string, ModelRowStatus | ModelRowFailure>;

export function ClaudeModelsEditor({
  rows,
  disabled = false,
  defaultModel = "",
  rowStatuses,
  onTestRow,
  onChange,
}: {
  rows: string[];
  disabled?: boolean;
  defaultModel?: string;
  rowStatuses?: ModelRowStatuses;
  onTestRow?: (index: number) => void;
  onChange: (rows: string[]) => void;
}) {
  const { t } = useI18n();
  const lastRowRef = useRef<HTMLInputElement | null>(null);
  const shouldFocusNewRow = useRef(false);

  useEffect(() => {
    if (shouldFocusNewRow.current) {
      shouldFocusNewRow.current = false;
      lastRowRef.current?.focus();
    }
  }, [rows.length]);

  const updateRow = (index: number, value: string) => {
    const next = [...rows];
    next[index] = value;
    onChange(next);
  };

  const removeRow = (index: number) => {
    const next = rows.filter((_, i) => i !== index);
    if (next.length === 0) {
      next.push("");
    }
    onChange(next);
  };

  const addRow = () => {
    onChange([...rows, ""]);
    shouldFocusNewRow.current = true;
  };

  const statusOf = (row: string): ModelRowStatus => {
    const status = rowStatuses?.[row.trim()];
    if (status == null) return "idle";
    if (typeof status === "string") return status;
    return "fail";
  };

  const failureOf = (row: string): string | null => {
    const status = rowStatuses?.[row.trim()];
    if (status == null || typeof status === "string") return null;
    return status.message;
  };

  return (
    <div className="space-y-2">
      {defaultModel.trim() !== "" && (
        <ModelRow
          label={t("settings.claude.defaultModelLabel")}
          value={defaultModel}
          badge={t("settings.claude.defaultModelBadge")}
          disabled
          status="ok"
        />
      )}
      {rows.map((row, index) => {
        const duplicate = isDuplicateRow(rows, index);
        return (
          <ModelRow
            key={index}
            label={t("settings.claude.modelRowLabel", { index: index + 1 })}
            removeLabel={t("settings.claude.removeModelRow", {
              index: index + 1,
            })}
            value={row}
            disabled={disabled}
            placeholder={
              defaultModel.trim() === ""
                ? t("settings.claude.customModelsHint")
                : ""
            }
            duplicate={duplicate}
            status={statusOf(row)}
            failure={failureOf(row)}
            testing={statusOf(row) === "testing"}
            onTest={onTestRow ? () => onTestRow(index) : undefined}
            onRemove={() => removeRow(index)}
            inputRef={index === rows.length - 1 ? lastRowRef : undefined}
            onChange={(value) => updateRow(index, value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                if (row.trim() !== "" && index === rows.length - 1) {
                  addRow();
                }
              }
            }}
          />
        );
      })}
      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={disabled}
        onClick={addRow}
      >
        <Plus className="size-3.5" />
        {t("settings.claude.addModel")}
      </Button>
    </div>
  );
}

function ModelRow({
  label,
  removeLabel,
  value,
  disabled,
  placeholder,
  badge,
  duplicate,
  status,
  failure,
  testing,
  inputRef,
  onTest,
  onRemove,
  onChange,
  onKeyDown,
}: {
  label: string;
  removeLabel?: string;
  value: string;
  disabled?: boolean;
  placeholder?: string;
  badge?: string;
  duplicate?: boolean;
  status: ModelRowStatus;
  failure?: string | null;
  testing?: boolean;
  inputRef?: React.Ref<HTMLInputElement>;
  onTest?: () => void;
  onRemove?: () => void;
  onChange?: (value: string) => void;
  onKeyDown?: (e: React.KeyboardEvent<HTMLInputElement>) => void;
}) {
  const { t } = useI18n();
  return (
    <div className="min-w-0 space-y-1">
      <div className="flex items-center gap-2">
        <Input
          ref={inputRef}
          value={value}
          disabled={disabled}
          readOnly={!onChange}
          onChange={(e) => onChange?.(e.target.value)}
          onKeyDown={onKeyDown}
          aria-label={label}
          placeholder={placeholder}
          className="min-w-0 flex-1 font-mono text-xs"
        />
        {status === "testing" && (
          <LoaderCircle
            className="size-3.5 shrink-0 animate-spin text-primary"
            aria-label={t("settings.claude.testingModel")}
          />
        )}
        {status === "ok" && (
          <CheckCircle2
            className="size-3.5 shrink-0 text-success"
            aria-label={t("settings.claude.modelAvailable")}
          />
        )}
        {status === "fail" && (
          <XCircle
            className="size-3.5 shrink-0 text-destructive"
            aria-label={t("settings.claude.modelUnavailable")}
          />
        )}
        {onTest && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-7 shrink-0 px-2 text-xs"
            disabled={disabled || testing || value.trim() === ""}
            onClick={onTest}
            title={t("settings.claude.testModelTitle")}
          >
            {testing
              ? t("settings.testing")
              : t("settings.claude.testModel")}
          </Button>
        )}
        {onRemove && (
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="shrink-0"
            disabled={disabled}
            onClick={onRemove}
            aria-label={removeLabel}
          >
            <X className="size-3.5" />
          </Button>
        )}
        {badge && (
          <span className="shrink-0 rounded-md border bg-muted/40 px-2 py-1 font-mono text-xs text-muted-foreground">
            {badge}
          </span>
        )}
      </div>
      {duplicate && (
        <p className="text-xs text-destructive">
          {t("settings.claude.duplicateModel")}
        </p>
      )}
      {status === "fail" && failure && (
        <p className="truncate text-xs text-destructive" title={failure}>
          {failure}
        </p>
      )}
    </div>
  );
}
