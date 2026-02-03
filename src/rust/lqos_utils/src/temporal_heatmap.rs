//! Rolling 15-minute heatmap storage with per-minute medians.

const RAW_SAMPLES: usize = 60;
const SUMMARY_BLOCKS: usize = 14;
const TOTAL_BLOCKS: usize = SUMMARY_BLOCKS + 1;

use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// Heatmap block medians for download, upload, RTT, and TCP retransmits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Allocative)]
pub struct HeatmapBlocks {
    /// Median download values for each block.
    pub download: [Option<f32>; TOTAL_BLOCKS],
    /// Median upload values for each block.
    pub upload: [Option<f32>; TOTAL_BLOCKS],
    /// Median RTT values for each block.
    pub rtt: [Option<f32>; TOTAL_BLOCKS],
    /// RTT 50th percentile (median) for download direction.
    #[serde(default)]
    pub rtt_p50_down: [Option<f32>; TOTAL_BLOCKS],
    /// RTT 50th percentile (median) for upload direction.
    #[serde(default)]
    pub rtt_p50_up: [Option<f32>; TOTAL_BLOCKS],
    /// RTT 90th percentile for download direction.
    #[serde(default)]
    pub rtt_p90_down: [Option<f32>; TOTAL_BLOCKS],
    /// RTT 90th percentile for upload direction.
    #[serde(default)]
    pub rtt_p90_up: [Option<f32>; TOTAL_BLOCKS],
    /// Median TCP retransmit percentage values for each block.
    pub retransmit: [Option<f32>; TOTAL_BLOCKS],
    /// TCP retransmit percentage for download direction.
    #[serde(default)]
    pub retransmit_down: [Option<f32>; TOTAL_BLOCKS],
    /// TCP retransmit percentage for upload direction.
    #[serde(default)]
    pub retransmit_up: [Option<f32>; TOTAL_BLOCKS],
}

/// Fixed-size rolling heatmap storage for 15 minutes of data.
///
/// Size: 4 * (RAW_SAMPLES + SUMMARY_BLOCKS) Option<f32> values + indices.
#[derive(Clone, Debug, Allocative)]
pub struct TemporalHeatmap {
    raw_download: [Option<f32>; RAW_SAMPLES],
    raw_upload: [Option<f32>; RAW_SAMPLES],
    raw_rtt: [Option<f32>; RAW_SAMPLES],
    raw_rtt_p50_down: [Option<f32>; RAW_SAMPLES],
    raw_rtt_p50_up: [Option<f32>; RAW_SAMPLES],
    raw_rtt_p90_down: [Option<f32>; RAW_SAMPLES],
    raw_rtt_p90_up: [Option<f32>; RAW_SAMPLES],
    raw_retransmit: [Option<f32>; RAW_SAMPLES],
    raw_retransmit_down: [Option<f32>; RAW_SAMPLES],
    raw_retransmit_up: [Option<f32>; RAW_SAMPLES],
    summary_download: [Option<f32>; SUMMARY_BLOCKS],
    summary_upload: [Option<f32>; SUMMARY_BLOCKS],
    summary_rtt: [Option<f32>; SUMMARY_BLOCKS],
    summary_rtt_p50_down: [Option<f32>; SUMMARY_BLOCKS],
    summary_rtt_p50_up: [Option<f32>; SUMMARY_BLOCKS],
    summary_rtt_p90_down: [Option<f32>; SUMMARY_BLOCKS],
    summary_rtt_p90_up: [Option<f32>; SUMMARY_BLOCKS],
    summary_retransmit: [Option<f32>; SUMMARY_BLOCKS],
    summary_retransmit_down: [Option<f32>; SUMMARY_BLOCKS],
    summary_retransmit_up: [Option<f32>; SUMMARY_BLOCKS],
    raw_index: usize,
    raw_filled: usize,
}

impl TemporalHeatmap {
    /// Create a new TemporalHeatmap with empty buffers.
    pub fn new() -> Self {
        const NONE_F32: Option<f32> = None;
        Self {
            raw_download: [NONE_F32; RAW_SAMPLES],
            raw_upload: [NONE_F32; RAW_SAMPLES],
            raw_rtt: [NONE_F32; RAW_SAMPLES],
            raw_rtt_p50_down: [NONE_F32; RAW_SAMPLES],
            raw_rtt_p50_up: [NONE_F32; RAW_SAMPLES],
            raw_rtt_p90_down: [NONE_F32; RAW_SAMPLES],
            raw_rtt_p90_up: [NONE_F32; RAW_SAMPLES],
            raw_retransmit: [NONE_F32; RAW_SAMPLES],
            raw_retransmit_down: [NONE_F32; RAW_SAMPLES],
            raw_retransmit_up: [NONE_F32; RAW_SAMPLES],
            summary_download: [NONE_F32; SUMMARY_BLOCKS],
            summary_upload: [NONE_F32; SUMMARY_BLOCKS],
            summary_rtt: [NONE_F32; SUMMARY_BLOCKS],
            summary_rtt_p50_down: [NONE_F32; SUMMARY_BLOCKS],
            summary_rtt_p50_up: [NONE_F32; SUMMARY_BLOCKS],
            summary_rtt_p90_down: [NONE_F32; SUMMARY_BLOCKS],
            summary_rtt_p90_up: [NONE_F32; SUMMARY_BLOCKS],
            summary_retransmit: [NONE_F32; SUMMARY_BLOCKS],
            summary_retransmit_down: [NONE_F32; SUMMARY_BLOCKS],
            summary_retransmit_up: [NONE_F32; SUMMARY_BLOCKS],
            raw_index: 0,
            raw_filled: 0,
        }
    }

    /// Add a single sample to the rolling buffer.
    pub fn add_sample(
        &mut self,
        download: f32,
        upload: f32,
        rtt_p50_down: Option<f32>,
        rtt_p50_up: Option<f32>,
        rtt_p90_down: Option<f32>,
        rtt_p90_up: Option<f32>,
        retransmit_down: Option<f32>,
        retransmit_up: Option<f32>,
    ) {
        let rtt = Self::combine_optional(rtt_p50_down, rtt_p50_up);
        let retransmit = Self::combine_optional(retransmit_down, retransmit_up);

        self.raw_download[self.raw_index] = Some(download);
        self.raw_upload[self.raw_index] = Some(upload);
        self.raw_rtt[self.raw_index] = rtt;
        self.raw_rtt_p50_down[self.raw_index] = rtt_p50_down;
        self.raw_rtt_p50_up[self.raw_index] = rtt_p50_up;
        self.raw_rtt_p90_down[self.raw_index] = rtt_p90_down;
        self.raw_rtt_p90_up[self.raw_index] = rtt_p90_up;
        self.raw_retransmit[self.raw_index] = retransmit;
        self.raw_retransmit_down[self.raw_index] = retransmit_down;
        self.raw_retransmit_up[self.raw_index] = retransmit_up;

        self.raw_index += 1;
        if self.raw_filled < RAW_SAMPLES {
            self.raw_filled += 1;
        }

        if self.raw_index == RAW_SAMPLES {
            self.push_summary_block();
            self.clear_raw_buffers();
        }
    }

    /// Return 15 blocks of median values for each tracked metric.
    pub fn blocks(&self) -> HeatmapBlocks {
        let mut download = [None; TOTAL_BLOCKS];
        let mut upload = [None; TOTAL_BLOCKS];
        let mut rtt = [None; TOTAL_BLOCKS];
        let mut rtt_p50_down = [None; TOTAL_BLOCKS];
        let mut rtt_p50_up = [None; TOTAL_BLOCKS];
        let mut rtt_p90_down = [None; TOTAL_BLOCKS];
        let mut rtt_p90_up = [None; TOTAL_BLOCKS];
        let mut retransmit = [None; TOTAL_BLOCKS];
        let mut retransmit_down = [None; TOTAL_BLOCKS];
        let mut retransmit_up = [None; TOTAL_BLOCKS];

        download[..SUMMARY_BLOCKS].copy_from_slice(&self.summary_download);
        upload[..SUMMARY_BLOCKS].copy_from_slice(&self.summary_upload);
        rtt[..SUMMARY_BLOCKS].copy_from_slice(&self.summary_rtt);
        rtt_p50_down[..SUMMARY_BLOCKS].copy_from_slice(&self.summary_rtt_p50_down);
        rtt_p50_up[..SUMMARY_BLOCKS].copy_from_slice(&self.summary_rtt_p50_up);
        rtt_p90_down[..SUMMARY_BLOCKS].copy_from_slice(&self.summary_rtt_p90_down);
        rtt_p90_up[..SUMMARY_BLOCKS].copy_from_slice(&self.summary_rtt_p90_up);
        retransmit[..SUMMARY_BLOCKS].copy_from_slice(&self.summary_retransmit);
        retransmit_down[..SUMMARY_BLOCKS].copy_from_slice(&self.summary_retransmit_down);
        retransmit_up[..SUMMARY_BLOCKS].copy_from_slice(&self.summary_retransmit_up);

        download[TOTAL_BLOCKS - 1] = Self::median_from_raw(&self.raw_download, self.raw_filled);
        upload[TOTAL_BLOCKS - 1] = Self::median_from_raw(&self.raw_upload, self.raw_filled);
        rtt[TOTAL_BLOCKS - 1] = Self::median_from_raw(&self.raw_rtt, self.raw_filled);
        rtt_p50_down[TOTAL_BLOCKS - 1] =
            Self::median_from_raw(&self.raw_rtt_p50_down, self.raw_filled);
        rtt_p50_up[TOTAL_BLOCKS - 1] = Self::median_from_raw(&self.raw_rtt_p50_up, self.raw_filled);
        rtt_p90_down[TOTAL_BLOCKS - 1] =
            Self::median_from_raw(&self.raw_rtt_p90_down, self.raw_filled);
        rtt_p90_up[TOTAL_BLOCKS - 1] = Self::median_from_raw(&self.raw_rtt_p90_up, self.raw_filled);
        retransmit[TOTAL_BLOCKS - 1] = Self::median_from_raw(&self.raw_retransmit, self.raw_filled);
        retransmit_down[TOTAL_BLOCKS - 1] =
            Self::median_from_raw(&self.raw_retransmit_down, self.raw_filled);
        retransmit_up[TOTAL_BLOCKS - 1] =
            Self::median_from_raw(&self.raw_retransmit_up, self.raw_filled);

        HeatmapBlocks {
            download,
            upload,
            rtt,
            rtt_p50_down,
            rtt_p50_up,
            rtt_p90_down,
            rtt_p90_up,
            retransmit,
            retransmit_down,
            retransmit_up,
        }
    }

    fn combine_optional(left: Option<f32>, right: Option<f32>) -> Option<f32> {
        match (left, right) {
            (Some(a), Some(b)) => Some((a + b) / 2.0),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    fn push_summary_block(&mut self) {
        let median_download = Self::median_from_raw(&self.raw_download, RAW_SAMPLES);
        let median_upload = Self::median_from_raw(&self.raw_upload, RAW_SAMPLES);
        let median_rtt = Self::median_from_raw(&self.raw_rtt, RAW_SAMPLES);
        let median_rtt_p50_down = Self::median_from_raw(&self.raw_rtt_p50_down, RAW_SAMPLES);
        let median_rtt_p50_up = Self::median_from_raw(&self.raw_rtt_p50_up, RAW_SAMPLES);
        let median_rtt_p90_down = Self::median_from_raw(&self.raw_rtt_p90_down, RAW_SAMPLES);
        let median_rtt_p90_up = Self::median_from_raw(&self.raw_rtt_p90_up, RAW_SAMPLES);
        let median_retransmit = Self::median_from_raw(&self.raw_retransmit, RAW_SAMPLES);
        let median_retransmit_down = Self::median_from_raw(&self.raw_retransmit_down, RAW_SAMPLES);
        let median_retransmit_up = Self::median_from_raw(&self.raw_retransmit_up, RAW_SAMPLES);

        Self::shift_summary(&mut self.summary_download, median_download);
        Self::shift_summary(&mut self.summary_upload, median_upload);
        Self::shift_summary(&mut self.summary_rtt, median_rtt);
        Self::shift_summary(&mut self.summary_rtt_p50_down, median_rtt_p50_down);
        Self::shift_summary(&mut self.summary_rtt_p50_up, median_rtt_p50_up);
        Self::shift_summary(&mut self.summary_rtt_p90_down, median_rtt_p90_down);
        Self::shift_summary(&mut self.summary_rtt_p90_up, median_rtt_p90_up);
        Self::shift_summary(&mut self.summary_retransmit, median_retransmit);
        Self::shift_summary(&mut self.summary_retransmit_down, median_retransmit_down);
        Self::shift_summary(&mut self.summary_retransmit_up, median_retransmit_up);
    }

    fn shift_summary(target: &mut [Option<f32>; SUMMARY_BLOCKS], value: Option<f32>) {
        for i in 1..SUMMARY_BLOCKS {
            target[i - 1] = target[i];
        }
        target[SUMMARY_BLOCKS - 1] = value;
    }

    fn clear_raw_buffers(&mut self) {
        self.raw_download.fill(None);
        self.raw_upload.fill(None);
        self.raw_rtt.fill(None);
        self.raw_rtt_p50_down.fill(None);
        self.raw_rtt_p50_up.fill(None);
        self.raw_rtt_p90_down.fill(None);
        self.raw_rtt_p90_up.fill(None);
        self.raw_retransmit.fill(None);
        self.raw_retransmit_down.fill(None);
        self.raw_retransmit_up.fill(None);
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

impl Default for TemporalHeatmap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{RAW_SAMPLES, SUMMARY_BLOCKS, TOTAL_BLOCKS, TemporalHeatmap};

    #[test]
    fn new_is_empty() {
        let heatmap = TemporalHeatmap::new();
        let blocks = heatmap.blocks();

        assert!(blocks.download.iter().all(|value| value.is_none()));
        assert!(blocks.upload.iter().all(|value| value.is_none()));
        assert!(blocks.rtt.iter().all(|value| value.is_none()));
        assert!(blocks.retransmit.iter().all(|value| value.is_none()));
        assert!(blocks.retransmit_down.iter().all(|value| value.is_none()));
        assert!(blocks.retransmit_up.iter().all(|value| value.is_none()));
    }

    #[test]
    fn add_sample_sets_current_block() {
        let mut heatmap = TemporalHeatmap::new();
        heatmap.add_sample(10.0, 20.0, Some(30.0), None, None, None, Some(1.0), Some(3.0));

        let blocks = heatmap.blocks();
        assert_eq!(blocks.download[TOTAL_BLOCKS - 1], Some(10.0));
        assert_eq!(blocks.upload[TOTAL_BLOCKS - 1], Some(20.0));
        assert_eq!(blocks.rtt[TOTAL_BLOCKS - 1], Some(30.0));
        assert_eq!(blocks.rtt_p50_down[TOTAL_BLOCKS - 1], Some(30.0));
        assert_eq!(blocks.rtt_p50_up[TOTAL_BLOCKS - 1], None);
        assert_eq!(blocks.retransmit[TOTAL_BLOCKS - 1], Some(2.0));
        assert_eq!(blocks.retransmit_down[TOTAL_BLOCKS - 1], Some(1.0));
        assert_eq!(blocks.retransmit_up[TOTAL_BLOCKS - 1], Some(3.0));
    }

    #[test]
    fn full_minute_pushes_summary() {
        let mut heatmap = TemporalHeatmap::new();
        for i in 1..=RAW_SAMPLES {
            let value = i as f32;
            heatmap.add_sample(value, value + 1.0, None, None, None, None, None, None);
        }

        let blocks = heatmap.blocks();
        assert_eq!(blocks.download[SUMMARY_BLOCKS - 1], Some(30.5));
        assert_eq!(blocks.upload[SUMMARY_BLOCKS - 1], Some(31.5));
        assert_eq!(blocks.download[TOTAL_BLOCKS - 1], None);
    }

    #[test]
    fn partial_minute_median() {
        let mut heatmap = TemporalHeatmap::new();
        for i in 1..=5 {
            let value = i as f32;
            heatmap.add_sample(value, value, Some(value), Some(value), None, None, None, None);
        }

        let blocks = heatmap.blocks();
        assert_eq!(blocks.download[TOTAL_BLOCKS - 1], Some(3.0));
        assert_eq!(blocks.upload[TOTAL_BLOCKS - 1], Some(3.0));
        assert_eq!(blocks.rtt[TOTAL_BLOCKS - 1], Some(3.0));
        assert_eq!(blocks.rtt_p50_down[TOTAL_BLOCKS - 1], Some(3.0));
        assert_eq!(blocks.rtt_p50_up[TOTAL_BLOCKS - 1], Some(3.0));
    }
}
