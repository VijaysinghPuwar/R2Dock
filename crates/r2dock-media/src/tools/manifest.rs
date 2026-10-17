//! The pinned FFmpeg packages: exact URLs and SHA-256 sums, per platform.
//!
//! The files are produced by `.github/workflows/tools.yml` and published as
//! assets of this repository's `tools-ffmpeg-<version>` release. Each binary
//! is zstd-compressed on its own. Two sums are pinned per binary: one for
//! the download (checked before unpacking) and one for the binary itself
//! (checked after unpacking and again before use).

/// One compressed binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    /// File name of the release asset (`*.zst`).
    pub file: &'static str,
    /// SHA-256 of the compressed download, lowercase hex.
    pub download_sha256: &'static str,
    /// SHA-256 of the unpacked binary, lowercase hex.
    pub binary_sha256: &'static str,
    /// Size of the compressed download, for progress.
    pub download_size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Package {
    /// `<os>-<arch>` as in `std::env::consts`.
    pub platform: &'static str,
    pub ffmpeg: Asset,
    pub ffprobe: Asset,
}

/// FFmpeg version of the pinned packages.
pub const VERSION: &str = "9.0.2";
/// Release that holds the packages. A new build gets a new release tag;
/// existing assets are never replaced.
pub const RELEASE_URL: &str =
    "https://github.com/VijaysinghPuwar/R2Dock/releases/download/tools-ffmpeg-9.0.2-1/";

/// The pinned packages. Filled in from the `tools` workflow output.
pub const PACKAGES: &[Package] = &[];

impl Package {
    pub fn total_download(&self) -> u64 {
        self.ffmpeg.download_size + self.ffprobe.download_size
    }
}

/// `<os>-<arch>` for the running system.
pub fn this_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The package for the running system, if there is one.
pub fn for_this_platform() -> Option<&'static Package> {
    find(PACKAGES, &this_platform())
}

pub fn find<'a>(packages: &'a [Package], platform: &str) -> Option<&'a Package> {
    packages.iter().find(|p| p.platform == platform)
}

/// The download URL of an asset. Only this fixed HTTPS location is used.
pub fn url(asset: &Asset) -> String {
    format!("{RELEASE_URL}{}", asset.file)
}
