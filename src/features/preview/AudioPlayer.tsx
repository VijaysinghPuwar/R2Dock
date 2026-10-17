import { Pause, Play, Volume2, VolumeX } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { IconButton } from "../../components/Button";
import "./preview.css";

function formatTime(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) return "0:00";
  const s = Math.floor(secs);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  const rs = r < 10 ? `0${r}` : String(r);
  if (h > 0) return `${h}:${m < 10 ? `0${m}` : m}:${rs}`;
  return `${m}:${rs}`;
}

export interface AudioPlayerProps {
  src: string;
  /** Used in accessible labels. */
  title: string;
  compact?: boolean;
}

/** Compact audio player: play/pause, scrubber, time, volume. */
export function AudioPlayer({ src, title, compact = false }: AudioPlayerProps) {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [playing, setPlaying] = useState(false);
  const [current, setCurrent] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setPlaying(false);
    setCurrent(0);
    setDuration(0);
    setFailed(false);
  }, [src]);

  useEffect(() => {
    const a = audioRef.current;
    if (!a) return;
    a.volume = volume;
    a.muted = muted;
  }, [volume, muted]);

  const toggle = () => {
    const a = audioRef.current;
    if (!a) return;
    if (a.paused) {
      a.play().catch(() => setFailed(true));
    } else {
      a.pause();
    }
  };

  const seek = (value: number) => {
    const a = audioRef.current;
    if (!a) return;
    a.currentTime = value;
    setCurrent(value);
  };

  return (
    <div className={compact ? "audio-player is-compact" : "audio-player"} role="group" aria-label={`Audio player: ${title}`}>
      <audio
        ref={audioRef}
        src={src}
        preload="metadata"
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => setPlaying(false)}
        onTimeUpdate={(e) => setCurrent(e.currentTarget.currentTime)}
        onLoadedMetadata={(e) => setDuration(e.currentTarget.duration)}
        onDurationChange={(e) => setDuration(e.currentTarget.duration)}
        onError={() => setFailed(true)}
      />
      <IconButton
        label={playing ? "Pause" : "Play"}
        icon={playing ? <Pause size={16} /> : <Play size={16} />}
        onClick={toggle}
        disabled={failed}
      />
      <input
        type="range"
        className="audio-range audio-scrubber"
        min={0}
        max={Number.isFinite(duration) && duration > 0 ? duration : 0}
        step={0.1}
        value={current}
        onChange={(e) => seek(Number(e.target.value))}
        aria-label="Seek"
        aria-valuetext={`${formatTime(current)} of ${formatTime(duration)}`}
        disabled={failed || !(duration > 0)}
      />
      <span className="audio-time small secondary num">
        {formatTime(current)} / {formatTime(duration)}
      </span>
      {!compact && (
        <>
          <IconButton
            label={muted ? "Unmute" : "Mute"}
            icon={muted || volume === 0 ? <VolumeX size={16} /> : <Volume2 size={16} />}
            onClick={() => setMuted((m) => !m)}
            disabled={failed}
          />
          <input
            type="range"
            className="audio-range audio-volume"
            min={0}
            max={1}
            step={0.05}
            value={muted ? 0 : volume}
            onChange={(e) => {
              setVolume(Number(e.target.value));
              setMuted(false);
            }}
            aria-label="Volume"
            disabled={failed}
          />
        </>
      )}
      {failed && <span className="small muted">Could not play this file.</span>}
    </div>
  );
}
