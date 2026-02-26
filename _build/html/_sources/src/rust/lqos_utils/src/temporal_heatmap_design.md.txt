# TemporalHeatmap Design

## Goals

- Keep a fixed-size, pre-allocated rolling window for 15 minutes of data.
- Store 60 raw samples (1 per second) for the current minute and 14 summary blocks for prior minutes.
- Track download, upload, RTT (optional), and TCP retransmit percentage (optional).
- Provide median values per 1-minute block for 15 blocks total.
- Avoid allocations in steady state; no persistence; optional use of Box for heap storage.

## Data Layout

Constants:

- RAW_SAMPLES = 60
- SUMMARY_BLOCKS = 14
- TOTAL_BLOCKS = 15

Fields (per instance):

- raw_download: [Option<f32>; RAW_SAMPLES]
- raw_upload: [Option<f32>; RAW_SAMPLES]
- raw_rtt: [Option<f32>; RAW_SAMPLES]
- raw_retransmit: [Option<f32>; RAW_SAMPLES]
- summary_download: [Option<f32>; SUMMARY_BLOCKS]
- summary_upload: [Option<f32>; SUMMARY_BLOCKS]
- summary_rtt: [Option<f32>; SUMMARY_BLOCKS]
- summary_retransmit: [Option<f32>; SUMMARY_BLOCKS]
- raw_index: usize (0..RAW_SAMPLES)
- raw_filled: usize (0..RAW_SAMPLES)

All arrays are initialized to None so partial minutes do not skew medians.

## Methods

Public:

- new() / default(): pre-allocate arrays and initialize indices.
- add_sample(download: f32, upload: f32,
  rtt_down: Option<f32>, rtt_up: Option<f32>,
  retransmit_down: Option<f32>, retransmit_up: Option<f32>)
  - Combine RTT and retransmit values:
    - If both present, use average.
    - If only one present, use that.
    - If neither present, store None.
  - Store download/upload and combined RTT/retransmit at raw_index.
  - Increment raw_index and raw_filled.
  - When raw_index reaches RAW_SAMPLES:
    - Compute median for each metric from raw_* (ignoring None values).
    - Shift summary_* left by one and append new median at the end.
    - Reset raw_* to None and raw_index/raw_filled to 0.
- blocks() -> HeatmapBlocks
  - Returns 15 blocks for each metric.
  - Blocks 0..SUMMARY_BLOCKS-1 mirror summary_* in oldest->newest order.
  - Block TOTAL_BLOCKS-1 is computed from raw_* via median (ignoring None).
  - If there are no samples for a block, return None for that slot.

Helpers:

- median(values: &mut [f32], len: usize) -> Option<f32>
  - Build a scratch buffer with present samples.
  - Sort and select middle element (or average of two middles).

## Output Type

- HeatmapBlocks { download: [Option<f32>; TOTAL_BLOCKS], upload: [Option<f32>; TOTAL_BLOCKS],
  rtt: [Option<f32>; TOTAL_BLOCKS], retransmit: [Option<f32>; TOTAL_BLOCKS] }

## Size and Memory Notes

- Fixed-size: 4 * (RAW_SAMPLES + SUMMARY_BLOCKS) = 296 Option<f32> values.
- Option<f32> is typically 8 bytes, so arrays are ~2368 bytes plus indices.
- Document the exact size formula in the Rust doc comment for TemporalHeatmap.
- Use arrays (or boxed arrays) to ensure no allocation growth after construction.

## Median Rules

- Missing samples (None) are ignored.
- If no samples exist for a block, return None for that block.
- For even counts, use the average of the two middle values.

