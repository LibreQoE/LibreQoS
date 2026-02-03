//! Rolling 15-minute heatmap storage for QoQ (0..100) scores.
//!
//! This mirrors the structure of `temporal_heatmap`, but stores four QoQ series:
//! - download_total, upload_total

const RAW_SAMPLES: usize = 60;
const SUMMARY_BLOCKS: usize = 14;
const TOTAL_BLOCKS: usize = SUMMARY_BLOCKS + 1;

use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// Heatmap block medians for QoQ (0..100) scores.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Allocative)]
pub struct QoqHeatmapBlocks {
    /// Per-minute medians for the download-direction QoO/QoQ score.
    ///
    /// This array has 15 blocks:
    /// - 14 completed minute blocks (oldest → newest)
    /// - 1 current in-progress minute block
    ///
    /// `None` means "no samples available for that block".
    pub download_total: [Option<f32>; TOTAL_BLOCKS],
    /// Per-minute medians for the upload-direction QoO/QoQ score.
    ///
    /// This array has 15 blocks:
    /// - 14 completed minute blocks (oldest → newest)
    /// - 1 current in-progress minute block
    ///
    /// `None` means "no samples available for that block".
    pub upload_total: [Option<f32>; TOTAL_BLOCKS],
}

/// Fixed-size rolling QoQ heatmap storage for 15 minutes of data.
#[derive(Clone, Debug, Allocative)]
pub struct TemporalQoqHeatmap {
    raw_download_total: [Option<f32>; RAW_SAMPLES],
    raw_upload_total: [Option<f32>; RAW_SAMPLES],
    summary_download_total: [Option<f32>; SUMMARY_BLOCKS],
    summary_upload_total: [Option<f32>; SUMMARY_BLOCKS],
    raw_index: usize,
    raw_filled: usize,
}

impl TemporalQoqHeatmap {
    /// Create an empty QoO/QoQ heatmap accumulator.
    pub fn new() -> Self {
        const NONE_F32: Option<f32> = None;
        Self {
            raw_download_total: [NONE_F32; RAW_SAMPLES],
            raw_upload_total: [NONE_F32; RAW_SAMPLES],
            summary_download_total: [NONE_F32; SUMMARY_BLOCKS],
            summary_upload_total: [NONE_F32; SUMMARY_BLOCKS],
            raw_index: 0,
            raw_filled: 0,
        }
    }

    /// Add one QoO/QoQ sample (typically called once per second).
    ///
    /// After 60 samples are added, the median of that 60-second window is pushed into the
    /// 14-minute summary, and the raw buffers are cleared for the next minute.
    pub fn add_sample(&mut self, download_total: Option<f32>, upload_total: Option<f32>) {
        self.raw_download_total[self.raw_index] = download_total;
        self.raw_upload_total[self.raw_index] = upload_total;

        self.raw_index += 1;
        if self.raw_filled < RAW_SAMPLES {
            self.raw_filled += 1;
        }

        if self.raw_index == RAW_SAMPLES {
            self.push_summary_block();
            self.clear_raw_buffers();
        }
    }

    /// Return heatmap blocks suitable for UI display.
    pub fn blocks(&self) -> QoqHeatmapBlocks {
        let mut download_total = [None; TOTAL_BLOCKS];
        let mut upload_total = [None; TOTAL_BLOCKS];

        download_total[..SUMMARY_BLOCKS].copy_from_slice(&self.summary_download_total);
        upload_total[..SUMMARY_BLOCKS].copy_from_slice(&self.summary_upload_total);

        download_total[TOTAL_BLOCKS - 1] =
            Self::median_from_raw(&self.raw_download_total, self.raw_filled);
        upload_total[TOTAL_BLOCKS - 1] = Self::median_from_raw(&self.raw_upload_total, self.raw_filled);

        QoqHeatmapBlocks {
            download_total,
            upload_total,
        }
    }

    fn push_summary_block(&mut self) {
        let median_download_total = Self::median_from_raw(&self.raw_download_total, RAW_SAMPLES);
        let median_upload_total = Self::median_from_raw(&self.raw_upload_total, RAW_SAMPLES);

        Self::shift_summary(&mut self.summary_download_total, median_download_total);
        Self::shift_summary(&mut self.summary_upload_total, median_upload_total);
    }

    fn shift_summary(target: &mut [Option<f32>; SUMMARY_BLOCKS], value: Option<f32>) {
        for i in 1..SUMMARY_BLOCKS {
            target[i - 1] = target[i];
        }
        target[SUMMARY_BLOCKS - 1] = value;
    }

    fn clear_raw_buffers(&mut self) {
        self.raw_download_total.fill(None);
        self.raw_upload_total.fill(None);
        self.raw_index = 0;
        self.raw_filled = 0;
    }

    fn median_from_raw(raw: &[Option<f32>; RAW_SAMPLES], filled: usize) -> Option<f32> {
        let filled = filled.min(RAW_SAMPLES);
        if filled == 0 {
            return None;
        }

        let mut values = [0.0f32; RAW_SAMPLES];
        let mut len = 0usize;
        for value in raw.iter().take(filled) {
            if let Some(sample) = value {
                values[len] = *sample;
                len += 1;
            }
        }

        if len == 0 {
            return None;
        }

        values[..len].sort_by(|a, b| a.total_cmp(b));
        let mid = len / 2;
        if len % 2 == 1 {
            Some(values[mid])
        } else {
            Some((values[mid - 1] + values[mid]) / 2.0)
        }
    }
}

impl Default for TemporalQoqHeatmap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{TOTAL_BLOCKS, TemporalQoqHeatmap};

    #[test]
    fn new_is_empty() {
        let heatmap = TemporalQoqHeatmap::new();
        let blocks = heatmap.blocks();
        assert!(blocks.download_total.iter().all(|v| v.is_none()));
        assert!(blocks.upload_total.iter().all(|v| v.is_none()));
    }

    #[test]
    fn add_sample_sets_current_block() {
        let mut heatmap = TemporalQoqHeatmap::new();
        heatmap.add_sample(Some(10.0), Some(20.0));
        let blocks = heatmap.blocks();
        assert_eq!(blocks.download_total[TOTAL_BLOCKS - 1], Some(10.0));
        assert_eq!(blocks.upload_total[TOTAL_BLOCKS - 1], Some(20.0));
    }
}
