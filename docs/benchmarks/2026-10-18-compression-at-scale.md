# Compression and uploads at scale, 2026-10-18 (0.4.1 plus fixes)

Question: what happens when a user queues about 100 long audiobooks (and also 1,000 small files) with compression on? Starting point, from one real upload: a 24.5-hour mono MP3 audiobook (48 kbps, 24 kHz, 530 MB) took 141 s to re-encode to the 32 kbps speech preset (353 MB), then uploaded in 22 parts.

Machine: Apple M5 MacBook (10 cores, 24 GB), macOS, Homebrew FFmpeg 9.0.1. Release builds. Uploads go to the in-process local S3 server; the test kit's `Network` limits request bodies to a shared bytes-per-second budget and can delay multipart aborts, to stand in for a slow or flaky uplink. All audio is synthetic (a tone plus noise, made with FFmpeg's `lavfi`); no real recordings were used. Numbers are from single runs.

Commands:

```sh
export R2DOCK_TEST_FFMPEG=/opt/homebrew/bin/ffmpeg
cargo test -p r2dock-transfer --release --test compress_scale -- --nocapture --test-threads=1
# realistic sizes (the book is made first, see below)
R2DOCK_SCALE_BOOK=/tmp/book.mp3 R2DOCK_SCALE_COUNT=6 R2DOCK_SCALE_BPS=2000000 \
  cargo test -p r2dock-transfer --release --test compress_scale -- --ignored --nocapture realistic_long
cargo test -p r2dock-transfer --release --test compress_scale -- --ignored --nocapture restart_with
cargo test -p r2dock-transfer --release --test queue_scale -- --ignored --nocapture
```

The 24.5-hour test book was made with:

```sh
ffmpeg -f lavfi -i "sine=f=220:r=24000:d=88200,volume=0.4[a];anoisesrc=r=24000:d=88200:a=0.05[n];[a][n]amix=inputs=2" \
  -c:a libmp3lame -b:a 48k -ac 1 -ar 24000 book.mp3
```

It is 529,200,524 bytes (generation took 268 s).

## Findings and fixes

| Area | Before | After |
|---|---|---|
| Compressed copies on disk | Encodes ran ahead of uploads without limit. 12 short books on a slow link: peak 9 copies in the work folder; with 100 books the peak grows toward 100 copies (about 35 GB at 353 MB each), then the free-space check starts sending originals | A new encode starts only while finished copies waiting for (or in) upload plus running encodes are fewer than upload slots plus encoders (3 + 1), and under 4 GiB. Same test: peak 4 copies |
| Uploads behind a long encode | The upload scheduler looked at only the first 256 queued jobs, including jobs waiting for the encoder. 50 small files queued behind 300 books waiting to compress did not start within 30 s (idle upload slots) | Jobs waiting for the encoder are skipped without counting. The 50 small files finished in 0.11 s |
| Pause all, 100 jobs mid-encode and mid-upload (6 multipart uploads, 8 MB/s link) | 14.7 s: multipart pause waited for every part in flight | 3.0 s: parts in flight get 3 s to finish, then stop (the rest is sent again on resume) |
| Pause, 3 multipart uploads on a 2 MB/s link | 66.6 s | 3.0 s; resume then completes and all bytes match |
| Cancel all, 100 jobs, abort answered after 5 s | 5.0 s (each job waited for its abort) | 21 ms; aborts still reach the server in the background (60 s limit) |
| Orphan FFmpeg or copies after pause or cancel | None found | None: no FFmpeg child process and an empty work folder after cancel (checked with `pgrep -P` and the folder) |
| Queue file for 100,000 queued jobs, draining 50,000 over about 15 s | `queue.json` (pretty JSON) rewritten up to once a second: 59.4 MB per save, about 636 MB written while draining | Compact snapshot plus append log: 50.1 MB first save, about 45 MB written while draining (one compaction; each finished job adds a few dozen bytes) |

Other changes from the same review, each with a test: a compressed copy is checked again before every upload attempt, including retries (a copy altered during a retry wait was uploaded before); a pause or cancel during that check is honored and stops the read between 1 MiB blocks; a job that finishes while it is being saved to the queue file can no longer come back after a restart; an "unchanged" compression result is saved, so a restart does not run the encode again; dropping a copy restores the original's scan identity; the copy check no longer depends on device numbers, which some file systems change after a restart; a finished download's record is written to disk before the file is moved into place.

## Realistic sizes

Six 24.5-hour books, uplink limited to 2,000,000 bytes/s, 1 encoder (the default), 3 upload slots:

| Measure | Result |
|---|---|
| Each encode, 529 MB to 352.8 MB (mp3-32k) | 193.3 to 213.0 s (193.4 s typical) |
| Total for 6 books | 1,359 s |
| Peak copies in the work folder | 2 (678 MB). Here each encode (about 194 s) was slower than each upload (about 176 s at 2 MB/s), so copies never piled up |
| FFmpeg processes at once | 1 |
| FFmpeg CPU while encoding | about 141% (of 1,000% on 10 cores), sampled once a second with `ps` |
| FFmpeg threads | at most 16 (`ps -M`); FFmpeg starts several threads even for one MP3 stream, but uses about 1.4 cores |
| Activity log rows | 18 for 6 uploads: `compress_started`, `compress_finished`, `transfer_completed` per file; no rows for progress |

This run used an earlier version of the test kit's link limit (each request waited for its share of the link before its body was read, instead of reading the body at the limited rate), so the total time is approximate; the encode figures do not depend on it. These encodes were slower than the 141 s in the original report. The test machine also ran builds part of the time, so treat 193 s as an upper figure for this machine, not a comparison.

Three 1-hour books (21.6 MB each), uplink limited to 600,000 bytes/s so uploads are slower than encodes:

| Measure | Result |
|---|---|
| Each encode, 21.6 MB to 14.4 MB (mp3-32k) | 8.2 s |
| Total for 3 books | 80.3 s |
| Peak copies in the work folder | 3 (43.2 MB), within the bound of upload slots plus encoders |
| FFmpeg processes at once | 1, about 124% CPU while encoding |
| Activity log rows | 9 for 3 uploads |

A longer run (eight 6-hour books) stopped making progress. The cause was a deadlock in the new queue log, not in the scheduler: when an append to `queue.log` failed (the `transfers` folder did not exist yet in a fresh data folder), the writer fell back to a full snapshot while still holding its own write lock. Appends now create the folder, the fallback releases the lock first, and two tests cover both cases (one fails within 10 s without the fix).

## Restart with many compressed copies

20 copies of 353 MiB (synthetic bytes, same content linked 20 times) held in the journal, then a restart and Resume on all:

| Measure | Result |
|---|---|
| Engine start (restore) | 0.39 ms: copies are not read at startup |
| One SHA-256 check of a 353 MiB copy | 0.64 s |
| First upload finished | 2.5 s after Resume |
| All 20 checked and uploaded (no link limit) | 14.4 s |
| Worst delay of a 10 ms timer on the async runtime during the checks | 3.1 ms (checks run on blocking threads) |

Each copy is checked right before its own upload (and again before each retry), so the cost is spread over the queue: at the 3-slot default, at most 3 checks run at once. Because the copies were linked, later reads likely came from the page cache; a cold read of 20 separate copies would take longer.

## CPU

- Encodes at once: 1 by default (setting 1 to 4). Uploads do not wait for encodes to finish; jobs waiting for the encoder hold no upload slot.
- FFmpeg now runs below normal priority on macOS and Linux too (`nice -n 10`, through the system `nice`, which replaces itself with FFmpeg); Windows already used `BELOW_NORMAL_PRIORITY_CLASS`. The effect under contention was not measured.
- FFmpeg threads are not limited by R2Dock. For MP3 audio the measured load is about 1.4 cores. Software video encoders can use every core per encode; this was not measured at scale here (see `2026-10-17-video-encoders.md`).

## Not covered

- A real network and real R2. The shaped local link limits request bodies only; latency, packet loss, and R2 throttling are not modeled.
- A cold-cache reread of many separate large copies.
- The free-space check was not driven to the point of a full disk in these runs; with the copy bound, a 100-book queue keeps at most 4 copies (about 1.4 GB at 353 MB) on disk.
