# Video encoders, 2026-10-17

Machine: Apple M5 MacBook, macOS, FFmpeg 9.0.1 (Homebrew build, for measurement only). Source: a 31 minute 1080p30 H.264 web video (3.6 Mbps video, 186 kbps AAC), itself made with Apple's hardware encoder. Quality is VMAF (libvmaf, `n_subsample=10`) against the source slice; 95 and above is indistinguishable for most viewers, below 90 shows visible loss.

## 60 second slice from the middle (video only)

| Encoder and setting | Size vs source | Video bitrate | VMAF | Encode time |
|---|---|---|---|---|
| H.264 VideoToolbox `-q:v 50` | 53.8% | 1.93 Mbps | 89.9 | 6.5 s |
| H.264 VideoToolbox `-q:v 60` | 89.3% | 3.21 Mbps | 94.8 | 6.4 s |
| H.264 VideoToolbox `-b:v 2M` | 55.7% | 2.00 Mbps | 88.8 | 6.8 s |
| HEVC VideoToolbox `-q:v 50` | 29.4% | 1.05 Mbps | 88.2 | 6.9 s |
| HEVC VideoToolbox `-q:v 60` | 51.1% | 1.83 Mbps | 93.4 | 6.9 s |
| **AV1 SVT-AV1 `-crf 35 -preset 8`** | **29.6%** | **1.06 Mbps** | **96.7** | 8.9 s |
| AV1 SVT-AV1 `-crf 40 -preset 8` | 21.8% | 0.78 Mbps | 96.3 | 9.3 s |
| x264 `-crf 23` (GPL, reference only) | 59.5% | 2.14 Mbps | 96.3 | 11.3 s |
| x264 `-crf 26` (GPL, reference only) | 37.7% | 1.35 Mbps | 94.8 | 10.1 s |

## Full file with R2Dock's pipeline (AV1 default)

936 MB to 285 MB (69.5% smaller) in 355 s (about 5.3 times real time), AV1 at 1.09 Mbps plus AAC 128 kbps. VMAF of the first 30 seconds against the original, frame aligned: 97.1.

## Decisions

- Default codec AV1 (SVT-AV1, CRF 35, preset 8): the smallest files at the highest quality measured, fast enough in software.
- "Most compatible" uses H.264 with the best hardware encoder at high quality (VideoToolbox `-q:v 60`). On already-compressed web video it saves little, so the 30 second sample test usually leaves such files unchanged; it pays off on high-bitrate phone recordings.
- HEVC is not offered: AV1 is smaller at better quality, and H.264 covers compatibility.

Measuring tip: compare from the start of both files, or cut the same slice first. Seeking both files by time can compare frames one apart (the source started at 0.021 s), which gave misleading scores of 75 to 78 for the same encode.
