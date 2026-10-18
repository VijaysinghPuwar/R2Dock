// Plain-language text for compression results (DESIGN.md, Compression).

import type { CompressionView } from "../../ipc/bindings/CompressionView";
import type { SkipReason } from "../../ipc/bindings/SkipReason";
import type { Unchanged } from "../../ipc/bindings/Unchanged";
import { formatBytes } from "../../lib/format";

function skipText(reason: SkipReason): string {
  switch (reason.reason) {
    case "video":
      return "Video is handled separately";
    case "videoOff":
      return "Video compression is off";
    case "hdr":
      return "HDR video kept as is";
    case "unsupportedStreams":
      return "Has tracks MP4 cannot hold";
    case "noVideoEncoder":
      return "No video encoder on this computer";
    case "alreadyEfficient":
      return `Already efficient (${reason.codec.toUpperCase()})`;
    case "alreadyCompressedByR2dock":
      return "Already compressed by R2Dock";
    case "tooSmall":
      return "Too small to compress";
    case "tooShort":
      return "Too short to compress";
    case "unknownDuration":
      return "Length could not be read";
    case "unsupportedFormat":
      return "Format not compressed in this mode";
    case "losslessKept":
      return "Lossless audio kept (setting)";
    case "multichannel":
      return `${reason.channels}-channel audio kept`;
    case "alreadyCompressed":
      return `Already compressed (${reason.kbps} kbps)`;
    case "savingTooSmall":
      return `Would save only ${reason.percent}%`;
    default:
      // A reason added in the core before this list knows it.
      return "Kept as is";
  }
}

/** Why a file was uploaded as it is. */
export function unchangedText(reason: Unchanged): string {
  switch (reason.kind) {
    case "notAudio":
      return "Not audio or video";
    case "plan":
      return skipText(reason.reason);
    case "sampleSavedTooLittle":
    case "resultSavedTooLittle":
      return `Would save only ${reason.percent}%`;
    case "noDiskSpace":
      return "Not enough disk space to compress";
    case "toolsMissing":
      return "Compression tools are not installed";
    default:
      return "Kept as is";
  }
}

/** One short line about compression for a transfer row, if any. */
export function compressionLine(view: CompressionView | null): string | null {
  if (!view) return null;
  switch (view.kind) {
    case "pending":
      return null;
    case "saved": {
      if (view.originalBytes <= 0) return null;
      const saved = view.originalBytes - view.bytes;
      return `Saved ${formatBytes(saved)} (${Math.round((saved / view.originalBytes) * 100)}%)`;
    }
    case "unchanged":
      return view.reason.kind === "notAudio" ? null : `Not compressed: ${unchangedText(view.reason)}`;
    case "failed":
      return "Compression failed; uploaded the original";
  }
}
