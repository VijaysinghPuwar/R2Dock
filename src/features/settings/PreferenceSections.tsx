import { useQueryClient } from "@tanstack/react-query";
import { ChevronRight, ExternalLink, FolderOpen, RotateCcw, Trash2 } from "lucide-react";
import { useState, type ReactNode } from "react";
import { applyAppearance } from "../../app/appearance";
import { qk, useAppInfoValue, usePreferences, useSettings, useUpdatePreferences } from "../../app/queries";
import { Button } from "../../components/Button";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { SegmentedControl, Switch } from "../../components/Controls";
import { Banner } from "../../components/Feedback";
import { Select } from "../../components/Fields";
import { LogoMark } from "../../components/Logo";
import { toast } from "../../components/toastStore";
import {
  errorMessage,
  historyClear,
  openExternal,
  openActivityFolder,
  openLogFolder,
  settingsChooseActivityDir,
  settingsChooseDownloadDir,
  settingsReset,
} from "../../ipc/commands";
import type { ConflictPolicy } from "../../ipc/bindings/ConflictPolicy";
import type { Density } from "../../ipc/bindings/Density";
import type { LogLevel } from "../../ipc/bindings/LogLevel";
import type { TextSize } from "../../ipc/bindings/TextSize";
import type { Theme } from "../../ipc/bindings/Theme";
import type { ViewMode } from "../../ipc/bindings/ViewMode";
import { NumberSetting } from "./NumberSetting";

export const SOURCE_URL = "https://github.com/VijaysinghPuwar/R2Dock";
export const LICENSE_URL = "https://github.com/VijaysinghPuwar/R2Dock/blob/main/LICENSE";

function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return (
    <section className="settings-section" aria-labelledby={`settings-${id}`}>
      <h2 id={`settings-${id}`} className="settings-heading">
        {title}
      </h2>
      <div className="settings-group">{children}</div>
    </section>
  );
}

function ControlRow({ label, helper, children }: { label: string; helper?: string; children: ReactNode }) {
  return (
    <div className="setting-row">
      <div className="setting-text">
        <span>{label}</span>
        {helper && <span className="small muted">{helper}</span>}
      </div>
      <div className="setting-control">{children}</div>
    </div>
  );
}

export function AppearanceSection() {
  const prefs = usePreferences();
  const update = useUpdatePreferences();

  const setTheme = (theme: Theme) => {
    applyAppearance({ ...prefs, theme });
    update({ theme });
  };
  const setDensity = (density: Density) => {
    applyAppearance({ ...prefs, density });
    update({ density });
  };
  const setTextSize = (textSize: TextSize) => {
    applyAppearance({ ...prefs, textSize });
    update({ textSize });
  };

  return (
    <Section id="appearance" title="Appearance">
      <ControlRow label="Theme">
        <SegmentedControl<Theme>
          label="Theme"
          value={prefs.theme}
          onChange={setTheme}
          options={[
            { value: "dark", label: "Dark" },
            { value: "paper", label: "Paper" },
            { value: "system", label: "System" },
          ]}
        />
      </ControlRow>
      <ControlRow label="Density">
        <SegmentedControl<Density>
          label="Density"
          value={prefs.density}
          onChange={setDensity}
          options={[
            { value: "comfortable", label: "Comfortable" },
            { value: "compact", label: "Compact" },
          ]}
        />
      </ControlRow>
      <ControlRow label="Text size">
        <SegmentedControl<TextSize>
          label="Text size"
          value={prefs.textSize}
          onChange={setTextSize}
          options={[
            { value: "default", label: "Default (13 px)" },
            { value: "large", label: "Large (14 px)" },
            { value: "larger", label: "Larger (15 px)" },
          ]}
        />
      </ControlRow>
      <Switch
        label="Show image thumbnails in grid view"
        description="Thumbnails download small images from R2 and may count toward usage."
        checked={prefs.showThumbnails}
        onChange={(showThumbnails) => update({ showThumbnails })}
      />
      <ControlRow label="Default view">
        <SegmentedControl<ViewMode>
          label="Default view"
          value={prefs.defaultView}
          onChange={(defaultView) => update({ defaultView })}
          options={[
            { value: "list", label: "List" },
            { value: "grid", label: "Grid" },
          ]}
        />
      </ControlRow>
    </Section>
  );
}

const CONCURRENCY_OPTIONS = [1, 2, 3, 4, 5, 6, 7, 8].map((n) => ({ value: String(n), label: String(n) }));
const CONFLICT_OPTIONS: Array<{ value: ConflictPolicy; label: string }> = [
  { value: "ask", label: "Ask" },
  { value: "skip", label: "Skip" },
  { value: "replace", label: "Replace" },
  { value: "keepBoth", label: "Keep both" },
];

function isConflictPolicy(v: string): v is ConflictPolicy {
  return CONFLICT_OPTIONS.some((o) => o.value === v);
}

export function TransfersSection() {
  const client = useQueryClient();
  const settings = useSettings();
  const prefs = usePreferences();
  const update = useUpdatePreferences();
  const [advanced, setAdvanced] = useState(false);
  const [choosing, setChoosing] = useState(false);

  const chooseDir = async () => {
    setChoosing(true);
    try {
      const view = await settingsChooseDownloadDir();
      client.setQueryData(qk.settings, view);
    } catch (e) {
      toast.danger(errorMessage(e));
    } finally {
      setChoosing(false);
    }
  };

  return (
    <Section id="transfers" title="Transfers">
      <Select
        label="Simultaneous transfers"
        fieldClassName="setting-select"
        value={String(prefs.concurrency)}
        options={CONCURRENCY_OPTIONS}
        onChange={(e) => update({ concurrency: Number.parseInt(e.target.value, 10) })}
      />
      <ControlRow label="Default download folder">
        <span className="mono truncate download-dir" title={settings.data?.downloadDir}>
          {settings.data?.downloadDir ?? "--"}
        </span>
        <Button size="sm" loading={choosing} onClick={() => void chooseDir()}>
          Change
        </Button>
      </ControlRow>
      <Switch
        label="Ask where to save each download"
        checked={prefs.askWhereToSave}
        onChange={(askWhereToSave) => update({ askWhereToSave })}
      />
      <Select
        label="When a file already exists"
        fieldClassName="setting-select"
        value={prefs.conflictPolicy}
        options={CONFLICT_OPTIONS}
        onChange={(e) => {
          const v = e.target.value;
          if (isConflictPolicy(v)) update({ conflictPolicy: v });
        }}
      />
      <NumberSetting
        label="Keep transfer history"
        unit="days"
        min={1}
        max={365}
        value={prefs.historyDays}
        onCommit={(historyDays) => update({ historyDays })}
      />
      <Switch
        label="Include hidden system files"
        description="Files like .DS_Store and Thumbs.db in uploaded folders."
        checked={prefs.includeHiddenOsFiles}
        onChange={(includeHiddenOsFiles) => update({ includeHiddenOsFiles })}
      />
      <button type="button" className="disclosure" aria-expanded={advanced} onClick={() => setAdvanced((v) => !v)}>
        <ChevronRight size={14} aria-hidden="true" />
        Advanced
      </button>
      {advanced && (
        <NumberSetting
          label="Upload in parts for files larger than"
          unit="MB"
          min={5}
          max={5000}
          value={prefs.multipartThresholdMib}
          onCommit={(multipartThresholdMib) => update({ multipartThresholdMib })}
        />
      )}
    </Section>
  );
}

export function SecuritySection() {
  const client = useQueryClient();
  const settings = useSettings();
  const prefs = usePreferences();
  const update = useUpdatePreferences();
  const [clearOpen, setClearOpen] = useState(false);

  const clear = async () => {
    try {
      await historyClear();
      setClearOpen(false);
      toast.success("Local history cleared.");
      await client.invalidateQueries({ queryKey: qk.history });
    } catch (e) {
      toast.danger(errorMessage(e));
    }
  };

  return (
    <Section id="security" title="Security">
      {settings.data && !settings.data.credentialStoreAvailable && (
        <div className="settings-banner">
          <Banner tone="warning">
            Your system keychain is not available. Keys are kept for this session only and you will be asked for the secret
            again next time.
          </Banner>
        </div>
      )}
      <p>
        Credentials are stored in: <strong>{settings.data?.credentialStore ?? "--"}</strong>
      </p>
      <p className="secondary">Deletes always ask for confirmation.</p>
      <NumberSetting
        label="Typed confirmation for deleting more than"
        unit="objects or any folder"
        min={1}
        max={100000}
        value={prefs.typedConfirmThreshold}
        onCommit={(typedConfirmThreshold) => update({ typedConfirmThreshold })}
      />
      <Switch
        label="Warn before opening downloaded files that can run programs"
        checked={prefs.warnExecutables}
        onChange={(warnExecutables) => update({ warnExecutables })}
      />
      <ControlRow label="Local history" helper="Recent files and transfer history on this computer.">
        <Button size="sm" icon={<Trash2 size={14} />} onClick={() => setClearOpen(true)}>
          Clear local history
        </Button>
      </ControlRow>
      <ConfirmDialog
        open={clearOpen}
        onOpenChange={setClearOpen}
        title="Clear local history?"
        body={<p>Recent files and transfer history are removed from this computer. Your files in R2 are not affected.</p>}
        confirmLabel="Clear"
        danger
        onConfirm={clear}
      />
    </Section>
  );
}

export function NotificationsSection() {
  const prefs = usePreferences();
  const update = useUpdatePreferences();
  return (
    <Section id="notifications" title="Notifications">
      <Switch
        label="Notify when a long transfer finishes in the background"
        checked={prefs.notifyOnComplete}
        onChange={(notifyOnComplete) => update({ notifyOnComplete })}
      />
      <Switch label="Notify on failures" checked={prefs.notifyOnFailure} onChange={(notifyOnFailure) => update({ notifyOnFailure })} />
    </Section>
  );
}

export function AdvancedSection() {
  const client = useQueryClient();
  const prefs = usePreferences();
  const update = useUpdatePreferences();
  const [resetOpen, setResetOpen] = useState(false);

  const reset = async () => {
    try {
      const view = await settingsReset();
      client.setQueryData(qk.settings, view);
      applyAppearance(view.preferences);
      setResetOpen(false);
      toast.success("Settings reset.");
    } catch (e) {
      toast.danger(errorMessage(e));
    }
  };

  const openLogs = () => {
    openLogFolder().catch((e: unknown) => toast.danger(errorMessage(e)));
  };
  const openActivity = () => {
    openActivityFolder().catch((e: unknown) => toast.danger(errorMessage(e)));
  };
  const settings = useSettings();
  const [choosingActivity, setChoosingActivity] = useState(false);
  const chooseActivityDir = async () => {
    setChoosingActivity(true);
    try {
      const view = await settingsChooseActivityDir();
      client.setQueryData(qk.settings, view);
    } catch (e) {
      toast.danger(errorMessage(e));
    } finally {
      setChoosingActivity(false);
    }
  };

  return (
    <Section id="advanced" title="Advanced">
      <Switch
        label="Activity log"
        description="Records each action and its result (with error codes) to help find problems. Never records keys or secrets."
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
      <ControlRow label="Activity log folder">
        <span className="mono truncate download-dir" title={settings.data?.activityDir}>
          {settings.data?.activityDir ?? "--"}
        </span>
        <Button size="sm" loading={choosingActivity} onClick={() => void chooseActivityDir()}>
          Change
        </Button>
        <Button size="sm" icon={<FolderOpen size={14} />} onClick={openActivity}>
          Open
        </Button>
      </ControlRow>
      <NumberSetting
        label="Keep activity logs for"
        unit="days"
        min={1}
        max={365}
        value={prefs.activityDays}
        onCommit={(activityDays) => update({ activityDays })}
      />
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
      <ControlRow label="Log files">
        <Button size="sm" icon={<FolderOpen size={14} />} onClick={openLogs}>
          Open log folder
        </Button>
      </ControlRow>
      <ControlRow label="Reset">
        <Button size="sm" icon={<RotateCcw size={14} />} onClick={() => setResetOpen(true)}>
          Reset all settings
        </Button>
      </ControlRow>
      <ConfirmDialog
        open={resetOpen}
        onOpenChange={setResetOpen}
        title="Reset all settings?"
        body={<p>Connections and keys are not affected.</p>}
        confirmLabel="Reset"
        danger
        onConfirm={reset}
      />
    </Section>
  );
}

export function AboutSection() {
  const info = useAppInfoValue();
  const open = (url: string) => {
    openExternal(url).catch((e: unknown) => toast.danger(errorMessage(e)));
  };
  return (
    <Section id="about" title="About">
      <div className="about-head">
        <LogoMark size={64} />
        <div>
          <p className="about-name">R2Dock</p>
          {info.version && <p className="secondary num">Version {info.version}</p>}
        </div>
      </div>
      <p>R2Dock collects no telemetry. It connects only to Cloudflare R2.</p>
      <div className="row">
        <Button size="sm" icon={<ExternalLink size={14} />} onClick={() => open(SOURCE_URL)}>
          Source code
        </Button>
        <Button size="sm" icon={<ExternalLink size={14} />} onClick={() => open(LICENSE_URL)}>
          License
        </Button>
      </div>
    </Section>
  );
}
