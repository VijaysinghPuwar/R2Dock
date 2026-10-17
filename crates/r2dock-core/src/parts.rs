//! Multipart part planning.
//!
//! R2 limits (PLAN.md 8.4): up to 10,000 parts, 5 MiB minimum part size
//! (except the last), 5 GiB maximum part size, all parts except the last the
//! same size.

use thiserror::Error;

pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;
pub const MIN_PART_SIZE: u64 = 5 * MIB;
pub const MAX_PART_SIZE: u64 = 5 * GIB;
pub const MAX_PARTS: u64 = 10_000;
/// Preferred minimum part size: keeps retries cheap and progress smooth.
pub const PREFERRED_PART_SIZE: u64 = 16 * MIB;
/// Leave headroom under the 10,000 part limit.
const PART_COUNT_TARGET: u64 = 9_000;
/// Largest object R2 accepts through multipart upload (about 5 TiB).
pub const MAX_OBJECT_SIZE: u64 = 5 * 1024 * GIB;
/// Largest single PUT.
pub const MAX_SINGLE_PUT: u64 = 5 * GIB;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PartError {
    #[error("File is too large for R2 (about 5 TiB maximum).")]
    TooLarge,
    #[error("Part size is outside R2 limits.")]
    InvalidPartSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartRange {
    /// 1-based part number, as S3 expects.
    pub number: i32,
    pub offset: u64,
    pub len: u64,
}

/// Choose a part size: `max(16 MiB, ceil(size / 9,000))`, rounded up to a whole MiB.
pub fn part_size_for(file_size: u64) -> Result<u64, PartError> {
    if file_size > MAX_OBJECT_SIZE {
        return Err(PartError::TooLarge);
    }
    let by_count = file_size.div_ceil(PART_COUNT_TARGET);
    let size = by_count.max(PREFERRED_PART_SIZE).div_ceil(MIB) * MIB;
    if size > MAX_PART_SIZE {
        return Err(PartError::TooLarge);
    }
    Ok(size)
}

/// Split a file into parts of `part_size` bytes (the last part may be smaller).
pub fn plan_parts(file_size: u64, part_size: u64) -> Result<Vec<PartRange>, PartError> {
    if !(MIN_PART_SIZE..=MAX_PART_SIZE).contains(&part_size) {
        return Err(PartError::InvalidPartSize);
    }
    let count = file_size.div_ceil(part_size).max(1);
    if count > MAX_PARTS {
        return Err(PartError::TooLarge);
    }
    let mut parts = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    let mut offset = 0;
    let mut number: i32 = 1;
    while offset < file_size || (file_size == 0 && number == 1) {
        let len = part_size.min(file_size - offset);
        parts.push(PartRange {
            number,
            offset,
            len,
        });
        offset += len;
        number += 1;
        if len == 0 {
            break;
        }
    }
    Ok(parts)
}

/// Whether a file should be uploaded in parts.
pub fn use_multipart(file_size: u64, threshold: u64) -> bool {
    file_size >= threshold.max(MIN_PART_SIZE) || file_size > MAX_SINGLE_PUT
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn small_files_use_preferred_size() {
        assert_eq!(part_size_for(0).unwrap(), 16 * MIB);
        assert_eq!(part_size_for(100 * MIB).unwrap(), 16 * MIB);
    }

    #[test]
    fn large_files_scale_part_size() {
        // 200 GiB / 9000 = about 22.76 MiB, rounded up to 23 MiB.
        assert_eq!(part_size_for(200 * GIB).unwrap(), 23 * MIB);
        let size = part_size_for(MAX_OBJECT_SIZE).unwrap();
        assert!(MAX_OBJECT_SIZE.div_ceil(size) <= MAX_PARTS);
        assert_eq!(part_size_for(MAX_OBJECT_SIZE + 1), Err(PartError::TooLarge));
    }

    #[test]
    fn plan_covers_file_exactly() {
        let parts = plan_parts(40 * MIB + 3, 16 * MIB).unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(
            parts[0],
            PartRange {
                number: 1,
                offset: 0,
                len: 16 * MIB
            }
        );
        assert_eq!(parts[2].len, 8 * MIB + 3);
        assert_eq!(plan_parts(16 * MIB, 16 * MIB).unwrap().len(), 1);
        assert_eq!(
            plan_parts(0, 16 * MIB).unwrap(),
            vec![PartRange {
                number: 1,
                offset: 0,
                len: 0
            }]
        );
        assert_eq!(plan_parts(10, MIB), Err(PartError::InvalidPartSize));
    }

    #[test]
    fn multipart_threshold() {
        assert!(!use_multipart(100 * MIB - 1, 100 * MIB));
        assert!(use_multipart(100 * MIB, 100 * MIB));
        assert!(use_multipart(100 * MIB + 1, 100 * MIB));
        assert!(!use_multipart(0, 100 * MIB));
        // A threshold below the 5 MiB minimum part size is raised to it.
        assert!(!use_multipart(MIB, 1));
    }

    proptest! {
        #[test]
        fn plans_are_contiguous_and_within_limits(size in 0u64..(64 * GIB)) {
            let part = part_size_for(size).unwrap();
            let parts = plan_parts(size, part).unwrap();
            prop_assert!(parts.len() as u64 <= MAX_PARTS);
            let mut expected_offset = 0;
            for (i, p) in parts.iter().enumerate() {
                prop_assert_eq!(p.number as usize, i + 1);
                prop_assert_eq!(p.offset, expected_offset);
                if i + 1 < parts.len() {
                    prop_assert_eq!(p.len, part);
                    prop_assert!(p.len >= MIN_PART_SIZE);
                }
                expected_offset += p.len;
            }
            prop_assert_eq!(expected_offset, size);
        }
    }
}
