# Fractional Speed Plans Implementation Plan

## Time Estimate (Human Hours)

**Total Estimated Time: 16-20 hours** (for a 10x developer)

### Phase Breakdown:
- **Phase 1 (Core Backend):** 4-5 hours
  - Rust data structure changes: 2 hours
  - Python parsing & TC logic: 2-3 hours
- **Phase 2 (TC Commands):** 1-2 hours  
  - Smart unit selection logic: 1-2 hours
- **Phase 3 (UI Updates):** 8-10 hours
  - Core input/validation changes: 2 hours
  - Rate display functions (scaling.js): 1-2 hours
  - 15+ UI component updates: 5-6 hours
- **Phase 4 (Integration):** 2-3 hours
  - UISP integration: 1 hour
  - Weight calculations: 1 hour
  - LTS/Insight compilation fixes: 1 hour

### Risk Buffer: +25% (4-5 additional hours)
- Debugging edge cases with small fractional rates
- UI testing across different rate ranges
- Ensuring TC command compatibility

**Notes:**
- Assumes familiarity with LibreQoS codebase architecture
- Time estimate includes testing but not extensive QA
- Future LTS/Insight full integration not included (separate project)

## Overview

This document outlines the plan to implement fractional speed plans in LibreQoS, allowing users to specify bandwidth rates in decimal format (e.g., 2.5 Mbps) instead of just whole numbers. The change will enable more granular bandwidth allocation for smaller plans.

## Current State Analysis

### Data Flow Summary
1. **CSV Input**: ShapedDevices.csv contains integer Mbps values
2. **Python Processing**: LibreQoS.py parses CSV and generates `tc` commands  
3. **Rust Storage**: lqos_config stores rates as `u32` integers
4. **UI Display**: Web interface shows and edits rates as integers
5. **TC Commands**: All rates passed to `tc` as "Xmbit" format

### Key Limitations
- All rate fields are integer-only (`u32` in Rust, `int()` in Python)
- CSV parsing expects whole numbers 
- UI inputs lack decimal support
- No unit conversion utilities
- TC command generation assumes integer Mbps

## Implementation Plan

### Phase 1: Core Data Structure Changes

#### 1.1 Rust Backend Changes
**Files to modify:**
- `rust/lqos_config/src/shaped_devices/shaped_device.rs`
- `rust/lqos_config/src/shaped_devices/serializable.rs`

**Changes:**
- Change rate fields from `u32` to `f32`:
  ```rust
  pub download_min_mbps: f32,
  pub upload_min_mbps: f32, 
  pub download_max_mbps: f32,
  pub upload_max_mbps: f32,
  ```
- Update CSV parsing to use `.parse::<f32>()`
- Add validation for positive decimal values
- Fix existing bugs in serializable.rs (lines 63, 86)

#### 1.2 Python LibreQoS.py Changes  
**File:** `LibreQoS.py`

**Changes:**
- Lines 273-303: Replace `int()` with `float()` parsing
- Lines 420-423, 454-457: Store as floats instead of integers
- Update validation logic to accept decimals ≥ 0.1 (instead of ≥ 1)
- Add intelligent TC unit selection logic:
  ```python
  def format_rate_for_tc(rate_mbps):
      if rate_mbps >= 1000:  # Use gbit for >= 1000 Mbps
          return f"{rate_mbps/1000:.1f}gbit"
      elif rate_mbps >= 1:    # Use mbit for >= 1 Mbps  
          return f"{rate_mbps:.1f}mbit"
      else:                   # Use kbit for < 1 Mbps
          return f"{rate_mbps*1000:.0f}kbit"
  ```

### Phase 2: TC Command Generation

#### 2.1 Smart Unit Selection
**Location:** LibreQoS.py lines 873, 880, 894, 901, 933, 937, 958, 966

**Logic:**
- **≥ 1000 Mbps**: Use `gbit` (e.g., "1.5gbit")
- **≥ 1 Mbps**: Use `mbit` (e.g., "2.5mbit") 
- **< 1 Mbps**: Use `kbit` (e.g., "500kbit")

**Benefits:**
- Maintains TC precision for small plans
- Avoids fractional kbit values 
- Preserves existing behavior for large plans

### Phase 3: UI Updates

#### 3.1 Frontend Input Changes
**Files:**
- `rust/lqosd/src/node_manager/js_build/src/config_devices.js`
- `rust/lqosd/src/node_manager/static2/config_devices.html`

**Changes:**
- Add `step="0.1"` to rate input fields (following existing UISP pattern)
- Replace `parseInt()` with `parseFloat()` validation
- Update minimum validation from 1 to 0.1
- Add decimal formatting for display

#### 3.2 Rate Display Updates - Core Components
**Files requiring rate display changes:**

**Primary Rate Display Functions:**
- `rust/lqosd/src/node_manager/js_build/src/helpers/scaling.js` (lines 35-45)
  - Update `formatThroughput()` to handle fractional rates
- `rust/lqosd/src/node_manager/js_build/src/lq_js_common/helpers/scaling.js` (lines 17-29)
  - Update `scaleNumber()` for decimal precision

**Circuit Page:**
- `rust/lqosd/src/node_manager/js_build/src/circuit.js` (lines 277-278, 386-391, 843)
  - Update bandwidth limits display ("+ / + Mbps" format)
  - Handle fractional rates in traffic tables

**Tree View:**
- `rust/lqosd/src/node_manager/js_build/src/tree.js` (lines 89-95, 162-168, 368, 372-373)
  - Update node bandwidth limit displays
  - Update device plan displays (down/up format)
  - Handle Mbps conversions with decimals

**Shaped Devices Page:**
- `rust/lqosd/src/node_manager/js_build/src/shaped-devices.js` (lines 32, 224-228)
  - Update plan display format for fractional rates

**Dashboard Components:**
- `rust/lqosd/src/node_manager/js_build/src/dashlets/throughput_bps_dash.js` (lines 267-275)
- `rust/lqosd/src/node_manager/js_build/src/dashlets/top10flows_rate.js` (lines 85-89)
- `rust/lqosd/src/node_manager/js_build/src/dashlets/top_tree_summary.js`
- `rust/lqosd/src/node_manager/js_build/src/graphs/bits_gauge.js` (lines 48, 52, 108, 112, 139-141)

**Capacity Dashboard Components:**
- `rust/lqosd/src/node_manager/js_build/src/dashlets/circuit_capacity_dash.js` (lines 46, 71, 90-91)
- `rust/lqosd/src/node_manager/js_build/src/dashlets/tree_capacity_dash.js` (lines 58-59, 61, 77-78)
- `rust/lqosd/src/node_manager/ws/ticker/circuit_capacity.rs` (lines 71, 73) - **Already handles f32 correctly**

**Changes:**
- Update all hardcoded "Mbps" strings to handle decimal display
- Ensure `scaleNumber()` and `formatThroughput()` preserve decimal precision
- Update gauge max value calculations for fractional rates
- Format rate displays to show decimals when present (e.g., "2.5 / 1.0 Mbps")

### Phase 4: Integration & Testing

#### 4.1 UISP Integration Updates
**File:** `rust/uisp_integration/src/strategies/full/shaped_devices_writer.rs`

**Changes:**
- Update existing fractional rate generation (lines 88-99) to use `f32` instead of truncating to `u64`
- Preserve fractional rates from UISP bandwidth calculations

#### 4.2 Weight Calculation Updates  
**File:** `rust/lqos_python/src/device_weights.rs`

**Changes:**
- Update line 79 to handle fractional rates in weight calculations
- Ensure proper rounding for weight values

### Phase 5: CSV Format & Migration

#### 5.1 CSV Header Updates
**File:** `ShapedDevices.example.csv`

**Changes:**
- Update column headers to indicate decimal support:
  - "Download Min Mbps (decimal allowed)"
  - "Upload Min Mbps (decimal allowed)"
  - "Download Max Mbps (decimal allowed)" 
  - "Upload Max Mbps (decimal allowed)"

#### 5.2 Backward Compatibility
- Existing integer CSV files will continue to work
- No migration required - floats parse integers correctly
- Validation ensures positive values only

### Phase 6: LTS and Insight Data Submission Updates

#### 6.1 Immediate Compilation Fixes (Required Now)
**Files requiring immediate attention:**

**LTS Data Submission:**
- `rust/lqosd/src/throughput_tracker/stats_submission.rs`
  - Update rate calculations to handle `f32` instead of `u32`
  - Ensure `ShapedDevice` serialization works with float rates

**Insight Data Submission:**
- `rust/lqosd/src/lts2_sys/shared_types.rs`
  - Update `ShapedDevice` references to use `f32` rates
  - Ensure CBOR serialization handles fractional rates correctly

**Immediate Workaround Strategy:**
```rust
// Temporary solution for data submission compatibility
fn rate_for_submission(rate_mbps: f32) -> u32 {
    if rate_mbps < 1.0 {
        1  // Round up small fractional rates to 1 Mbps for now
    } else {
        rate_mbps.round() as u32  // Round to nearest integer
    }
}
```

**Changes Required:**
- Update all `ShapedDevice` usage in submission code
- Add temporary rounding functions to prevent data loss
- Ensure backward compatibility with existing LTS/Insight consumers

#### 6.2 Future LTS Integration (Full Implementation)
**Status:** Future work requiring schema changes

**Long-term Considerations:**
- **Database Schema Updates:** LTS database tables storing rate data need decimal precision
- **Historical Data Migration:** Existing integer rate data vs new fractional rates  
- **Aggregation Logic:** Rate averaging and analysis with decimal precision
- **API Changes:** LTS query APIs need to handle fractional rate filters
- **Performance Impact:** Decimal storage and calculation efficiency

**Data Structures Affected:**
```rust
// Current LTS structures that will need updates:
ThroughputSummary {
    // Rate plan data from ShapedDevice gets submitted here
    shaped_bits_per_second: (u64, u64),
}

CircuitThroughput {
    // Circuit rate limits affect capacity calculations  
    download_bytes: u64,
    upload_bytes: u64,
}
```

#### 6.3 Future Insight Integration (Full Implementation)  
**Status:** Future work requiring protocol changes

**Long-term Considerations:**
- **Protocol Updates:** Insight API protocol needs fractional rate support
- **Rate Monitoring:** Alerting and thresholds with decimal precision
- **Capacity Planning:** Analysis tools with granular rate data
- **Performance Metrics:** Utilization calculations with fractional bandwidth targets
- **Visualization:** Charts and graphs displaying fractional rates

**Data Submission Changes:**
```rust
// Future Insight structures needing fractional support:
ShaperThroughput {
    // Configured rate limits from ShapedDevice
    bytes_per_second_down: i64,
    bytes_per_second_up: i64,
}

// Rate plan configuration data
ShapedDeviceConfig {
    download_min_mbps: f32,  // Future: fractional rates
    upload_min_mbps: f32,
    download_max_mbps: f32, 
    upload_max_mbps: f32,
}
```

#### 6.4 Data Integrity Strategy
**Preventing Data Loss During Transition:**

1. **Submission Compatibility:** Use rounding functions for external systems during transition
2. **Internal Precision:** Maintain full fractional precision within LibreQoS
3. **Gradual Migration:** Update external consumers before removing rounding workarounds
4. **Validation:** Ensure fractional rates don't break existing analysis tools
5. **Fallback Handling:** Graceful degradation for systems that can't handle decimals

## Implementation Order

1. **Start with Rust backend** - Core data structure changes
2. **Update Python parsing** - CSV and validation logic  
3. **Implement TC formatting** - Smart unit selection
4. **Update UI components** - Input and display changes
5. **Integration testing** - End-to-end validation
6. **Documentation updates** - CSV examples and user guides

## Risk Mitigation

### Precision Concerns
- Use `f32` for reasonable precision (6-7 decimal digits)
- Validate against extremely small values (< 0.01 Mbps)
- Round appropriately for TC commands

### Backward Compatibility
- Existing integer CSV files remain valid
- UI displays work with both integer and decimal rates
- TC commands maintain existing format for integer rates

### Testing Strategy
- Unit tests for rate parsing and validation
- Integration tests for TC command generation  
- UI tests for decimal input handling
- Performance tests with fractional rates

## Success Criteria

1. Users can enter rates like "2.5" or "0.5" in CSV and UI
2. TC commands use appropriate units (mbit/kbit/gbit)  
3. All existing integer rates continue to work unchanged
4. UI properly validates and displays fractional rates
5. Rate calculations maintain precision throughout the system
6. No performance degradation with fractional rates

## Files Requiring Changes

### High Priority (Core Implementation)
- `LibreQoS.py` - CSV parsing and TC generation
- `rust/lqos_config/src/shaped_devices/shaped_device.rs` - Data structures
- `rust/lqosd/src/node_manager/js_build/src/config_devices.js` - UI inputs
- `rust/lqosd/src/throughput_tracker/stats_submission.rs` - LTS compilation fix
- `rust/lqosd/src/lts2_sys/shared_types.rs` - Insight compilation fix

### Medium Priority (UI/Integration)  
- `rust/lqosd/src/node_manager/js_build/src/shaped-devices.js` - Display
- `rust/lqosd/src/node_manager/static2/config_devices.html` - HTML forms
- `rust/lqos_python/src/device_weights.rs` - Weight calculations

### Medium Priority (UI Rate Display Components)
**Core Display Functions:**
- `rust/lqosd/src/node_manager/js_build/src/helpers/scaling.js` - formatThroughput()
- `rust/lqosd/src/node_manager/js_build/src/lq_js_common/helpers/scaling.js` - scaleNumber()

**Page Components:**
- `rust/lqosd/src/node_manager/js_build/src/circuit.js` - Circuit rate displays
- `rust/lqosd/src/node_manager/js_build/src/tree.js` - Tree view rate displays
- `rust/lqosd/src/node_manager/js_build/src/dashlets/throughput_bps_dash.js` - Dashboard
- `rust/lqosd/src/node_manager/js_build/src/dashlets/top10flows_rate.js` - Top flows
- `rust/lqosd/src/node_manager/js_build/src/dashlets/top_tree_summary.js` - Tree summary
- `rust/lqosd/src/node_manager/js_build/src/graphs/bits_gauge.js` - Rate gauges
- `rust/lqosd/src/node_manager/js_build/src/helpers/builders.js` - Table builders

### Low Priority (Future/Optional)
- `ShapedDevices.example.csv` - Documentation
- Full LTS/Insight fractional rate support (future work)
- Additional validation and error handling
- Rate unit conversion utilities
- Performance optimization for decimal calculations

This plan provides a structured approach to implementing fractional speed plans while maintaining backward compatibility and system reliability.