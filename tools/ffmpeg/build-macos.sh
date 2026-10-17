#!/usr/bin/env bash
# Build a static LGPL v3 FFmpeg for macOS (the architecture of this machine)
# with LAME (LGPL), Opus (BSD), and SVT-AV1 (BSD), for R2Dock's media
# compression. H.264 and HEVC use the system's VideoToolbox.
#
# Used by .github/workflows/tools.yml. Every source is pinned by SHA-256.
# No GPL or nonfree components are enabled.
#
# Usage: tools/ffmpeg/build-macos.sh <work dir> <output dir>
set -euo pipefail

# Absolute paths, since the build changes directory.
mkdir -p "$1" "$2"
WORK="$(cd "$1" && pwd)"
OUT="$(cd "$2" && pwd)"
export MACOSX_DEPLOYMENT_TARGET=11.0

FFMPEG_URL="https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz"
FFMPEG_SHA="8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e"
LAME_URL="https://downloads.sourceforge.net/project/lame/lame/3.100/lame-3.100.tar.gz"
LAME_SHA="ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e"
OPUS_URL="https://downloads.xiph.org/releases/opus/opus-1.5.2.tar.gz"
OPUS_SHA="65c1d2f78b9f2fb20082c38cbe47c951ad5839345876e46941612ee87f9a7ce1"
SVTAV1_URL="https://gitlab.com/AOMediaCodec/SVT-AV1/-/archive/v4.2.0/SVT-AV1-v4.2.0.tar.gz"
SVTAV1_SHA="c7b13c4a84bd3751aa35fcc72be13e6875467e7c2216879251a486e5b1e4e740"

PREFIX="$WORK/prefix"
JOBS="$(sysctl -n hw.ncpu)"

fetch() {
  local url="$1" sha="$2" file
  file="$WORK/$(basename "$url")"
  curl -fsSL --retry 3 -o "$file" "$url"
  echo "$sha  $file" | shasum -a 256 -c -
  tar -xf "$file" -C "$WORK"
}

fetch "$LAME_URL" "$LAME_SHA"
fetch "$OPUS_URL" "$OPUS_SHA"
fetch "$SVTAV1_URL" "$SVTAV1_SHA"
fetch "$FFMPEG_URL" "$FFMPEG_SHA"

(
  cd "$WORK/lame-3.100"
  # The symbol list ships an entry that no longer exists; recent linkers fail on it.
  sed -i '' '/lame_init_old/d' include/libmp3lame.sym
  ./configure --prefix="$PREFIX" --disable-shared --enable-static --disable-frontend --disable-dependency-tracking
  make -j"$JOBS" install
)

(
  cd "$WORK/opus-1.5.2"
  ./configure --prefix="$PREFIX" --disable-shared --enable-static --disable-doc --disable-extra-programs --disable-dependency-tracking
  make -j"$JOBS" install
)

(
  mkdir -p "$WORK/SVT-AV1-v4.2.0/build-r2dock"
  cd "$WORK/SVT-AV1-v4.2.0/build-r2dock"
  cmake .. -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$PREFIX" \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOSX_DEPLOYMENT_TARGET" \
    -DBUILD_SHARED_LIBS=OFF -DBUILD_APPS=OFF -DBUILD_TESTING=OFF -DSVT_AV1_LTO=OFF
  make -j"$JOBS" install
)

(
  cd "$WORK/ffmpeg-9.0.2"
  PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig" ./configure \
    --prefix="$PREFIX" \
    --pkg-config-flags=--static \
    --extra-cflags="-I$PREFIX/include" \
    --extra-ldflags="-L$PREFIX/lib" \
    --enable-version3 \
    --disable-shared --enable-static \
    --disable-autodetect \
    --enable-videotoolbox --enable-audiotoolbox --enable-zlib \
    --enable-libmp3lame --enable-libopus --enable-libsvtav1 \
    --disable-ffplay --disable-doc --disable-network --disable-debug
  make -j"$JOBS"
  cp ffmpeg ffprobe "$OUT/"
  cp COPYING.LGPLv3 "$OUT/LICENSE-FFmpeg.txt"
)
cp "$WORK/lame-3.100/COPYING" "$OUT/LICENSE-LAME.txt"
cp "$WORK/opus-1.5.2/COPYING" "$OUT/LICENSE-Opus.txt"
cat "$WORK/SVT-AV1-v4.2.0/LICENSE.md" "$WORK/SVT-AV1-v4.2.0/PATENTS.md" > "$OUT/LICENSE-SVT-AV1.txt"

# Only system libraries may be linked.
for bin in ffmpeg ffprobe; do
  if otool -L "$OUT/$bin" | tail -n +2 | grep -vE '^\s+(/usr/lib/|/System/Library/)'; then
    echo "error: $bin links a non-system library" >&2
    exit 1
  fi
done
"$OUT/ffmpeg" -hide_banner -version
"$OUT/ffmpeg" -hide_banner -version | grep -q -- "--enable-gpl" && { echo "error: GPL build" >&2; exit 1; }
"$OUT/ffmpeg" -hide_banner -encoders | grep -E "libmp3lame|libopus|libsvtav1| aac | flac |videotoolbox"
"$OUT/ffmpeg" -hide_banner -encoders | grep -q libsvtav1 || { echo "error: no AV1 encoder" >&2; exit 1; }
