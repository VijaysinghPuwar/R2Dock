import { useQueryClient } from "@tanstack/react-query";
import { Copy, Download, FolderOpen, RotateCcw } from "lucide-react";
import { useState, type ReactNode } from "react";
import { qk, usePreferences, useSettings, useUpdatePreferences } from "../../app/queries";
import { Button } from "../../components/Button";
import { SegmentedControl, Switch } from "../../components/Controls";
import { Banner } from "../../components/Feedback";
import { Select } from "../../components/Fields";
import { toast } from "../../components/toastStore";
import {
  activityExportCsv,
  activityResetDir,
  errorMessage,
  logCopyPath,
  openActivityFolder,
  openLogFolder,
  settingsChooseActivityDir,
} from "../../ipc/commands";
import type { ExportRange } from "../../ipc/bindings/ExportRange";
import type { LogKind } from "../../ipc/bindings/LogKind";
import type { LogLevel } from "../../ipc/bindings/LogLevel";
import type { SettingsView } from "../../ipc/bindings/SettingsView";
import { NumberSetting } from "./NumberSetting";
import { ControlRow, Section } from "./SettingsLayout";

const RANGE_OPTIONS: Array<{ value: ExportRange; label: string }> = [
  { value: "today", label: "Today" },
  { value: "last7Days", label: "Last 7 days" },
  { value: "last30Days", label: "Last 30 days" },
  { value: "all", label: "All" },
];

function isExportRange(v: string): v is ExportRange {
  return RANGE_OPTIONS.some((o) => o.value === v);
}

function LogLocation({
  label,
  path,
  kind,
  children,
}: {
  label: string;
  path: string | undefined;
  kind: LogKind;
  children?: ReactNode;
}) {
  const open = () => {
    const call = kind === "activity" ? openActivityFolder() : openLogFolder();
    call.catch((e: unknown) => toast.danger(errorMessage(e)));
  };
  const copy = () => {
    logCopyPath(kind)
      .then(() => toast.success("Path copied."))
      .catch((e: unknown) => toast.danger(errorMessage(e)));
  };
  return (
    <div className="log-location">
      <span>{label}</span>
      <span className="mono small log-location-path selectable">{path ?? "--"}</span>
      <div className="log-location-actions">
        <Button size="sm" icon={<FolderOpen size={14} />} onClick={open}>
          Open folder
        </Button>
        <Button size="sm" icon={<Copy size={14} />} onClick={copy}>
          Copy path
        </Button>
        {children}
      </div>
    </div>
  );
}

/** Settings > Advanced > Logs: where the logs are and how to get them. */
export function LogsSection() {
  const client = useQueryClient();
  const settings = useSettings();
  const prefs = usePreferences();
  const update = useUpdatePreferences();
  const [choosing, setChoosing] = useState(false);
  const [range, setRange] = useState<ExportRange>("last7Days");
  const [exporting, setExporting] = useState(false);

  const changeView = async (call: () => Promise<SettingsView>) => {
    try {
      client.setQueryData(qk.settings, await call());
    } catch (e) {
      toast.danger(errorMessage(e));
    }
  };
  const chooseDir = async () => {
    setChoosing(true);
    await changeView(settingsChooseActivityDir);
    setChoosing(false);
  };
  const exportCsv = async () => {
    setExporting(true);
    try {
      const rows = await activityExportCsv(range);
      if (rows !== null) toast.success(rows === 1 ? "Exported 1 row." : `Exported ${rows.toLocaleString()} rows.`);
    } catch (e) {
      toast.danger(errorMessage(e));
    } finally {
      setExporting(false);
    }
  };

  return (
    <Section id="logs" title="Logs">
      {settings.data?.activityLogProblem && (
        <div className="settings-banner">
          <Banner tone="warning">{settings.data.activityLogProblem}</Banner>
        </div>
      )}
      <Switch
        label="Activity log"
        description="Records each action and its result (with error codes) to help find problems. Never records keys or secrets. Saved as CSV files that open in Excel, Numbers, or Google Sheets."
        checked={prefs.activityLog}
        onChange={(activityLog) => update({ activityLog })}
      />
      <Switch
        label="Include file and folder names in the activity log"
        description="Off: names are replaced with short codes. Turn on only while tracking down a problem."
        checked={prefs.activityIncludeNames}
        disabled={!prefs.activityLog}
        onChange={(activityIncludeNames) => update({ activityIncludeNames })}
      />
      <NumberSetting
        label="Keep activity logs for"
        unit="days"
        min={1}
        max={365}
        value={prefs.activityDays}
        onCommit={(activityDays) => update({ activityDays })}
      />
      <LogLocation label="Activity log location" path={settings.data?.activityDir} kind="activity">
        <Button size="sm" loading={choosing} onClick={() => void chooseDir()}>
          Change...
        </Button>
        {settings.data && !settings.data.activityDirIsDefault && (
          <Button size="sm" icon={<RotateCcw size={14} />} onClick={() => void changeView(activityResetDir)}>
            Reset to default
          </Button>
        )}
      </LogLocation>
      <div className="setting-row setting-row-end">
        <Select
          label="Export activity"
          fieldClassName="setting-select"
          value={range}
          options={RANGE_OPTIONS}
          onChange={(e) => {
            const v = e.target.value;
            if (isExportRange(v)) setRange(v);
          }}
        />
        <Button size="sm" icon={<Download size={14} />} loading={exporting} onClick={() => void exportCsv()}>
          Export as CSV...
        </Button>
      </div>
      <LogLocation label="Technical log location" path={settings.data?.logDir} kind="technical" />
      <ControlRow label="Log level" helper="Logs never contain your secret keys.">
        <SegmentedControl<LogLevel>
          label="Log level"
          value={prefs.logLevel}
          onChange={(logLevel) => update({ logLevel })}
          options={[
            { value: "standard", label: "Standard" },
            { value: "detailed", label: "Detailed" },
          ]}
        />
      </ControlRow>
    </Section>
  );
}
