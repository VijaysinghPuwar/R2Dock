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

/// The pinned packages, from the `MANIFEST.txt` of the release above.
/// Platforms match `std::env::consts` (`<os>-<arch>`).
pub const PACKAGES: &[Package] = &[
    Package {
        platform: "windows-x86_64",
        ffmpeg: Asset {
            file: "ffmpeg-9.0.2-windows-x86_64.exe.zst",
            download_sha256: "55fade02f3c45a86b4c4c8117c1e1ba76865189b34fd33bb8db2d6fefbd03bbd",
            binary_sha256: "0e482b853f2fd4821055d20cdce510f0eea6027037c04b994f454a2136e8868e",
            download_size: 45_444_949,
        },
        ffprobe: Asset {
            file: "ffprobe-9.0.2-windows-x86_64.exe.zst",
            download_sha256: "c8af3c5abac2aa7dd278759ca1534020d80646b65b27dc0e23e28d1bfff09409",
            binary_sha256: "b9627185ffbad1d4fc6ef953868ca9ad8df75fc8dd035fcab58aca78caa2d23e",
            download_size: 45_351_194,
        },
    },
    Package {
        platform: "linux-x86_64",
        ffmpeg: Asset {
            file: "ffmpeg-9.0.2-linux-x86_64.zst",
            download_sha256: "10999eecea699c2594bbdaaa25ce3fa790bd5cfadefc515888cfb5f29c8759a5",
            binary_sha256: "953632dece404bb1461a4a2285f0191a6105d72c81f9da577eb54326a7fad49a",
            download_size: 48_708_732,
        },
        ffprobe: Asset {
            file: "ffprobe-9.0.2-linux-x86_64.zst",
            download_sha256: "258a88a731c668c89796d68c65bd22a1f3c0c7468c9694127865353e5f7b35ee",
            binary_sha256: "c0d4ee234730e7b083cc729108ac34f3b718cc4858c3cf8ed6e0dcec545f13c1",
            download_size: 48_613_437,
        },
    },
    Package {
        platform: "macos-aarch64",
        ffmpeg: Asset {
            file: "ffmpeg-9.0.2-macos-aarch64.zst",
            download_sha256: "272fa8899ba0f1833ea831d34f38e0d156ee41c66bfc9cd0ddf492d9ad0d615d",
            binary_sha256: "a3c80c908c32059a92bfbeeb2d25dcaaed0e762098a2c7dceea7236f04067c06",
            download_size: 8_813_112,
        },
        ffprobe: Asset {
            file: "ffprobe-9.0.2-macos-aarch64.zst",
            download_sha256: "6403f3395e3ec66fe02c3b0c51fdd2f954ef9422021e29c94b72d5866583aefa",
            binary_sha256: "583ddcf9afc92a5ba483eb235261033fb40359b2b163cdda782db65b36b0019f",
            download_size: 8_733_877,
        },
    },
    Package {
        platform: "macos-x86_64",
        ffmpeg: Asset {
            file: "ffmpeg-9.0.2-macos-x86_64.zst",
            download_sha256: "9f76e17abb64c6ffdd7f1d0c5cbc70a597568a6976a69ae9ec992c12aeb1aac0",
            binary_sha256: "1f3aa8d921f252ac6130a24c3cab3bfde42457274284599f9cbaf6341e018661",
            download_size: 9_558_320,
        },
        ffprobe: Asset {
            file: "ffprobe-9.0.2-macos-x86_64.zst",
            download_sha256: "467bf08a8786e71efd67fb6e428a63a134487256e6d7573de44d74ed6dfc559e",
            binary_sha256: "fd4518291d5f4f6ff2cab3d7343ac776eec7d6af9fc0cdb5633d2a980746d455",
            download_size: 9_471_306,
        },
    },
];

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
