import { useQuery, useQueryClient } from "@tanstack/react-query";
import { AudioLines, Download, FolderOpen, Trash2, X } from "lucide-react";
import { useState } from "react";
import { qk, usePreferences, useUpdatePreferences } from "../../app/queries";
import { Button } from "../../components/Button";
import { SegmentedControl, Switch } from "../../components/Controls";
import { Banner } from "../../components/Feedback";
import { Select } from "../../components/Fields";
import { ProgressBar } from "../../components/ProgressBar";
import { toast } from "../../components/toastStore";
import {
  compressionCancelInstall,
  compressionInstall,
  compressionPreview,
  compressionRemove,
  compressionStatus,
  compressionUseDownloaded,
  compressionUseSystem,
  errorMessage,
  isAppError,
} from "../../ipc/commands";
import type { AudioSettings } from "../../ipc/bindings/AudioSettings";
import type { CompressionFailure } from "../../ipc/bindings/CompressionFailure";
import type { CompressionPrefs } from "../../ipc/bindings/CompressionPrefs";
import type { CompressionStatus } from "../../ipc/bindings/CompressionStatus";
import type { License } from "../../ipc/bindings/License";
import type { LosslessAction } from "../../ipc/bindings/LosslessAction";
import type { Mode } from "../../ipc/bindings/Mode";
import type { MusicQuality } from "../../ipc/bindings/MusicQuality";
import type { PreviewSample } from "../../ipc/bindings/PreviewSample";
import type { SpeechQuality } from "../../ipc/bindings/SpeechQuality";
import type { VideoCodec } from "../../ipc/bindings/VideoCodec";
import type { VideoSettings } from "../../ipc/bindings/VideoSettings";
import type { VideoSize } from "../../ipc/bindings/VideoSize";
import { formatBytes } from "../../lib/format";
import { mediaUrl } from "../../lib/media";
import { NumberSetting } from "./NumberSetting";
import { ControlRow, Section } from "./SettingsLayout";

const LICENSE_TEXT: Record<License, string> = {
  lgplV21: "LGPL 2.1",
  lgplV3: "LGPL 3",
  gplV2: "GPL 2",
  gplV3: "GPL 3",
  nonfree: "non-free",
};

const ENCODE_OPTIONS = [1, 2, 3, 4].map((n) => ({ value: String(n), label: String(n) }));
const FAILURE_OPTIONS: Array<{ value: CompressionFailure; label: string }> = [
  { value: "uploadOriginal", label: "Upload the original" },
  { value: "skipFile", label: "Skip the file" },
];

const SIZE_OPTIONS: Array<{ value: VideoSize; label: string }> = [
  { value: "small720", label: "Small (720p)" },
  { value: "balanced1080", label: "Balanced (1080p)" },
  { value: "keep", label: "Keep resolution" },
  { value: "off", label: "Don't compress video" },
];

function isVideoSize(v: string): v is VideoSize {
  return SIZE_OPTIONS.some((o) => o.value === v);
}

function isFailure(v: string): v is CompressionFailure {
  return FAILURE_OPTIONS.some((o) => o.value === v);
}

/** The FFmpeg block: not installed, installing, ready, or unusable. */
function ToolsBlock({ status }: { status: CompressionStatus | undefined }) {
  const client = useQueryClient();
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [working, setWorking] = useState(false);
  const set = (s: CompressionStatus) => client.setQueryData(qk.compression, s);

  const install = async () => {
    setProgress({ done: 0, total: status?.downloadBytes ?? 0 });
    try {
      set(await compressionInstall((p) => setProgress({ done: p.done, total: p.total })));
      toast.success("Compression tools installed.");
    } catch (e) {
      if (!(isAppError(e) && e.code === "cancelled")) toast.danger(errorMessage(e));
      void client.invalidateQueries({ queryKey: qk.compression });
    } finally {
      setProgress(null);
    }
  };
  const run = async (call: () => Promise<CompressionStatus | null>) => {
    setWorking(true);
    try {
      const s = await call();
      if (s) set(s);
    } catch (e) {
      toast.danger(errorMessage(e));
    } finally {
      setWorking(false);
    }
  };

  if (!status) return <p className="secondary">Checking the compression tools...</p>;

  if (progress) {
    const pct = progress.total > 0 ? Math.floor((progress.done / progress.total) * 100) : 0;
    return (
      <div className="tools-block">
        <p>
          Downloading compression tools: {formatBytes(progress.done)} of {formatBytes(progress.total)}
        </p>
        <ProgressBar value={progress.done} max={progress.total || 1} label={`${pct}%`} />
        <div className="log-location-actions">
          <Button size="sm" icon={<X size={14} />} onClick={() => void compressionCancelInstall()}>
            Cancel
          </Button>
        </div>
      </div>
    );
  }

  if (status.state === "ready") {
    const license = status.license ? ` (${LICENSE_TEXT[status.license]})` : "";
    return (
      <div className="tools-block">
        <p>
          FFmpeg {status.version}
          {license}, {status.source === "system" ? "from your system" : "downloaded and checked by R2Dock"}.
        </p>
        {status.source === "system" && status.systemPath && <p className="mono small log-location-path">{status.systemPath}</p>}
        <div className="log-location-actions">
          {status.source === "system" ? (
            <Button size="sm" loading={working} onClick={() => void run(compressionUseDownloaded)}>
              Use downloaded tools
            </Button>
          ) : (
            <Button size="sm" icon={<Trash2 size={14} />} loading={working} onClick={() => void run(compressionRemove)}>
              Remove tools
            </Button>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="tools-block">
      {status.state === "unusable" && status.problem && (
        <Banner tone="warning">The chosen FFmpeg cannot be used: {status.problem}</Banner>
      )}
      <p>
        {status.downloadBytes !== null
          ? `Compression tools are not installed (${formatBytes(status.downloadBytes)} download).`
          : "No compression tools are available to download for this system."}
      </p>
      <div className="log-location-actions">
        {status.downloadBytes !== null && (
          <Button size="sm" variant="primary" icon={<Download size={14} />} onClick={() => void install()}>
            Install compression tools
          </Button>
        )}
        <Button size="sm" icon={<FolderOpen size={14} />} loading={working} onClick={() => void run(compressionUseSystem)}>
          Use FFmpeg from my system...
        </Button>
      </div>
    </div>
  );
}

/** "Compare quality": 30 second samples of one file at each preset. */
function ComparePreview({ ready }: { ready: boolean }) {
  const [samples, setSamples] = useState<PreviewSample[] | null>(null);
  const [busy, setBusy] = useState(false);
  const choose = async () => {
    setBusy(true);
    try {
      const result = await compressionPreview();
      if (result) setSamples(result);
    } catch (e) {
      toast.danger(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="log-location">
      <span>Compare quality</span>
      <span className="small muted">
        Listen to 30 seconds of one of your files at each setting before choosing.
      </span>
      <div className="log-location-actions">
        <Button size="sm" icon={<AudioLines size={14} />} loading={busy} disabled={!ready} onClick={() => void choose()}>
          Choose a file...
        </Button>
      </div>
      {samples && (
        <ul className="preview-samples">
          {samples.map((s) => (
            <li key={s.token} className="preview-sample">
              <span className="preview-sample-label">
                {s.label}
                <span className="small muted"> about {formatBytes(s.bytesPerHour)} per hour</span>
              </span>
              <audio controls preload="none" src={mediaUrl(s.token)} aria-label={`${s.label} sample`} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** Settings > Compression (docs/plans/media-compression.md). */
export function CompressionSection() {
  const prefs = usePreferences();
  const update = useUpdatePreferences();
  const status = useQuery({ queryKey: qk.compression, queryFn: compressionStatus });
  const c = prefs.compression;
  const setC = (patch: Partial<CompressionPrefs>) => update({ compression: { ...c, ...patch } });
  const setAudio = (patch: Partial<AudioSettings>) => setC({ audio: { ...c.audio, ...patch } });
  const setVideo = (patch: Partial<VideoSettings>) => setC({ video: { ...c.video, ...patch } });

  return (
    <Section id="compression" title="Compression">
      <Switch
        label="Compress audio and video before upload"
        description="Makes audio (MP3, M4A, WAV, FLAC) and video (MP4, MOV, MKV) smaller on your computer before they upload. Your original files are never changed, and files that would not get smaller upload as they are."
        checked={c.enabled}
        onChange={(enabled) => setC({ enabled })}
      />
      <ToolsBlock status={status.data} />
      <ComparePreview ready={status.data?.state === "ready"} />
      <ControlRow label="Format" helper={c.audio.mode === "smallest" ? "Speech becomes Opus (.opus); file names change." : "MP3 stays MP3 and M4A stays M4A, so links keep working."}>
        <SegmentedControl<Mode>
          label="Format"
          value={c.audio.mode}
          onChange={(mode) => setAudio({ mode })}
          options={[
            { value: "keepFormat", label: "Keep format" },
            { value: "smallest", label: "Smallest" },
          ]}
        />
      </ControlRow>
      <ControlRow label="Speech quality" helper="Audiobooks, podcasts, and lectures.">
        <SegmentedControl<SpeechQuality>
          label="Speech quality"
          value={c.audio.speech}
          onChange={(speech) => setAudio({ speech })}
          options={[
            { value: "kbps32", label: "32 kbps" },
            { value: "kbps48", label: "48 kbps" },
          ]}
        />
      </ControlRow>
      <ControlRow label="Music quality">
        <SegmentedControl<MusicQuality>
          label="Music quality"
          value={c.audio.music}
          onChange={(music) => setAudio({ music })}
          options={[
            { value: "standard", label: "Standard" },
            { value: "smaller", label: "Smaller" },
          ]}
        />
      </ControlRow>
      <ControlRow label="WAV and other lossless audio">
        <SegmentedControl<LosslessAction>
          label="WAV and other lossless audio"
          value={c.audio.lossless}
          onChange={(lossless) => setAudio({ lossless })}
          options={[
            { value: "mp3", label: c.audio.mode === "smallest" ? "Opus" : "MP3" },
            { value: "flac", label: "FLAC (lossless)" },
            { value: "keep", label: "Keep" },
          ]}
        />
      </ControlRow>
      <ControlRow
        label="Video"
        helper={
          c.video.codec === "av1"
            ? "About a third of the size. Plays in Chrome, Edge, Firefox, and Safari on recent Apple devices."
            : "Plays everywhere. Saves less on videos that are already compressed."
        }
      >
        <SegmentedControl<VideoCodec>
          label="Video"
          value={c.video.codec}
          onChange={(codec) => setVideo({ codec })}
          options={[
            { value: "av1", label: "Smallest file (AV1)" },
            { value: "h264", label: "Most compatible (H.264)" },
          ]}
        />
      </ControlRow>
      <Select
        label="Video size"
        fieldClassName="setting-select"
        value={c.video.size}
        options={SIZE_OPTIONS}
        helper="The shorter side of the picture, so portrait phone videos are treated the same."
        onChange={(e) => {
          const v = e.target.value;
          if (isVideoSize(v)) setVideo({ size: v });
        }}
      />
      <NumberSetting
        label="Minimum saving"
        unit="%"
        min={5}
        max={50}
        value={c.audio.minSavingPercent}
        helper="Files that would shrink less than this upload as they are."
        onCommit={(minSavingPercent) => setAudio({ minSavingPercent })}
      />
      <Select
        label="If compression fails"
        fieldClassName="setting-select"
        value={c.onFailure}
        options={FAILURE_OPTIONS}
        onChange={(e) => {
          const v = e.target.value;
          if (isFailure(v)) setC({ onFailure: v });
        }}
      />
      <Select
        label="Files compressed at the same time"
        fieldClassName="setting-select"
        value={String(c.encodeLimit)}
        options={ENCODE_OPTIONS}
        onChange={(e) => setC({ encodeLimit: Number.parseInt(e.target.value, 10) })}
      />
    </Section>
  );
}
