# Fractional Speed Plans - Implementation Steps

This document breaks down the fractional speed plans implementation into specific tasks with tests for tracking progress.

## Step 1: Core Rust Data Structure Changes
**Estimated Time:** 2 hours  
**Priority:** Critical (blocks all other work)

### Task 1.1: Update ShapedDevice Struct
**File:** `rust/lqos_config/src/shaped_devices/shaped_device.rs`

**Changes:**
- [ ] Change `download_min_mbps: u32` to `download_min_mbps: f32` (line ~44)
- [ ] Change `upload_min_mbps: u32` to `upload_min_mbps: f32` (line ~45)  
- [ ] Change `download_max_mbps: u32` to `download_max_mbps: f32` (line ~46)
- [ ] Change `upload_max_mbps: u32` to `upload_max_mbps: f32` (line ~47)

**Test 1.1:**
```bash
cd rust/lqos_config
cargo check
# Should compile without errors
```

### Task 1.2: Update CSV Parsing
**File:** `rust/lqos_config/src/shaped_devices/shaped_device.rs`

**Changes:**
- [ ] Update line ~83: `record[8].parse::<f32>()` for download_min_mbps
- [ ] Update line ~86: `record[9].parse::<f32>()` for upload_min_mbps
- [ ] Update line ~89: `record[10].parse::<f32>()` for download_max_mbps
- [ ] Update line ~92: `record[11].parse::<f32>()` for upload_max_mbps
- [ ] Add validation for positive values > 0.01

**Test 1.2:**
```bash
# Create test CSV with fractional rates
echo '"Circuit ID","Circuit Name","Device ID","Device Name","Parent Node","MAC","IPv4","IPv6","Download Min Mbps","Upload Min Mbps","Download Max Mbps","Upload Max Mbps","Comment"' > test_fractional.csv
echo '"test1","Test Circuit","device1","Test Device","site1","00:00:00:00:00:01","192.168.1.1","","0.5","1.0","2.5","3.0","Test"' >> test_fractional.csv

# Test parsing (create small Rust test)
cargo test
```

### Task 1.3: Fix Serialization Bugs
**File:** `rust/lqos_config/src/shaped_devices/serializable.rs`

**Changes:**
- [ ] Fix line 63: Return constructed buffer instead of `String::new()`
- [ ] Fix line 86: Return constructed buffer instead of `String::new()`
- [ ] Update serialization to handle f32 rates

**Test 1.3:**
```bash
cargo test serializable
# All serialization tests should pass
```

## Step 2: Python LibreQoS.py Changes
**Estimated Time:** 2-3 hours
**Priority:** Critical

### Task 2.1: Update CSV Parsing and Validation
**File:** `LibreQoS.py`

**Changes:**
- [ ] Lines 273-303: Replace `int(downloadMin)` with `float(downloadMin)`
- [ ] Lines 273-303: Replace `int(uploadMin)` with `float(uploadMin)`
- [ ] Lines 273-303: Replace `int(downloadMax)` with `float(downloadMax)`
- [ ] Lines 273-303: Replace `int(uploadMax)` with `float(uploadMax)`
- [ ] Update validation: minimum rates ≥ 0.1 instead of ≥ 1
- [ ] Update validation: maximum rates ≥ 0.2 instead of ≥ 2

**Test 2.1:**
```bash
# Test with fractional CSV
cp ShapedDevices.example.csv ShapedDevices.test.csv
# Edit to add fractional rates like 0.5, 1.5, 2.5
python3 LibreQoS.py --dry-run
# Should parse without validation errors
```

### Task 2.2: Update Data Storage
**File:** `LibreQoS.py`

**Changes:**
- [ ] Lines 420-423: Store as floats instead of ints
- [ ] Lines 454-457: Store as floats instead of ints

**Test 2.2:**
```bash
# Inspect generated network.json
python3 LibreQoS.py --dry-run
cat network.json | grep -A5 -B5 "minDownload\|maxDownload"
# Should show decimal values preserved
```

### Task 2.3: Smart TC Unit Selection
**File:** `LibreQoS.py`

**Changes:**
- [ ] Create new function `format_rate_for_tc(rate_mbps)`:
  ```python
  def format_rate_for_tc(rate_mbps):
      if rate_mbps >= 1000:
          return f"{rate_mbps/1000:.1f}gbit"
      elif rate_mbps >= 1:
          return f"{rate_mbps:.1f}mbit"
      else:
          return f"{rate_mbps*1000:.0f}kbit"
  ```
- [ ] Update TC command generation (lines 873, 880, 894, 901, 933, 937, 958, 966)
- [ ] Replace hardcoded "mbit" with function calls

**Test 2.3:**
```bash
# Test TC command generation
python3 LibreQoS.py --dry-run
# Check generated commands in debug output:
# - Rates >= 1000 Mbps should use "gbit"
# - Rates >= 1 Mbps should use "mbit" 
# - Rates < 1 Mbps should use "kbit"

# Test specific cases:
# 0.5 Mbps -> "500kbit"
# 2.5 Mbps -> "2.5mbit"  
# 1500 Mbps -> "1.5gbit"
```

## Step 3: LTS/Insight Compilation Fixes
**Estimated Time:** 1 hour
**Priority:** High (needed for compilation)

### Task 3.1: Add Temporary Rounding Functions
**Files:** 
- `rust/lqosd/src/throughput_tracker/stats_submission.rs`
- `rust/lqosd/src/lts2_sys/shared_types.rs`

**Changes:**
- [ ] Add helper function:
  ```rust
  fn rate_for_submission(rate_mbps: f32) -> u32 {
      if rate_mbps < 1.0 {
          1  // Round up small rates to 1 Mbps temporarily
      } else {
          rate_mbps.round() as u32
      }
  }
  ```
- [ ] Update all `ShapedDevice` rate usage to use this function
- [ ] Add TODO comments for future fractional support

**Test 3.1:**
```bash
cd rust/lqosd
cargo check
# Should compile without errors

# Test data submission doesn't crash
cargo test stats_submission
cargo test shared_types
```

## Step 4: Core UI Input Changes
**Estimated Time:** 2 hours
**Priority:** High

### Task 4.1: Update Device Configuration Form
**File:** `rust/lqosd/src/node_manager/js_build/src/config_devices.js`

**Changes:**
- [ ] Add `step="0.1"` to rate input generation (lines 35-37)
- [ ] Replace `parseInt()` with `parseFloat()` (lines 167-170)
- [ ] Update validation: minimum 0.1 instead of 1
- [ ] Update `makeSheetNumberBox()` calls for rate fields

**Test 4.1:**
```javascript
// Browser test:
// 1. Open config_devices.html
// 2. Try entering "2.5" in rate fields
// 3. Should accept and validate correctly
// 4. Should save fractional values to CSV
```

### Task 4.2: Update HTML Form Attributes
**File:** `rust/lqosd/src/node_manager/static2/config_devices.html`

**Changes:**
- [ ] Add `step="0.1"` to any hardcoded rate inputs
- [ ] Update placeholder text to show decimal examples

**Test 4.2:**
```html
<!-- Manual test in browser -->
<!-- Rate inputs should allow decimal entry -->
<!-- Step buttons should increment by 0.1 -->
```

## Step 5: Core Display Function Updates
**Estimated Time:** 1-2 hours
**Priority:** High

### Task 5.1: Update Rate Formatting Functions
**File:** `rust/lqosd/src/node_manager/js_build/src/helpers/scaling.js`

**Changes:**
- [ ] Update `formatThroughput()` (lines 35-45) to preserve decimal precision
- [ ] Ensure rate displays show decimals when present

**File:** `rust/lqosd/src/node_manager/js_build/src/lq_js_common/helpers/scaling.js`

**Changes:**
- [ ] Update `scaleNumber()` (lines 17-29) for proper decimal formatting
- [ ] Ensure fractional rates display correctly

**Test 5.1:**
```javascript
// Unit tests for display functions
console.log(formatThroughput(2500000)); // Should show "2.5 Mbps"
console.log(scaleNumber(2.5, 1)); // Should preserve decimal
```

## Step 6: UI Component Updates (Multiple Files)
**Estimated Time:** 5-6 hours
**Priority:** Medium

### Task 6.1: Circuit Page Updates
**File:** `rust/lqosd/src/node_manager/js_build/src/circuit.js`

**Changes:**
- [ ] Lines 277-278: Update traffic table rate display
- [ ] Lines 386-391: Update device throughput display  
- [ ] Line 843: Update bandwidth limits display format

**Test 6.1:**
```javascript
// Browser test:
// 1. Navigate to circuit page
// 2. Check bandwidth displays show fractional rates correctly
// 3. Traffic table should format rates properly
```

### Task 6.2: Tree View Updates
**File:** `rust/lqosd/src/node_manager/js_build/src/tree.js`

**Changes:**
- [ ] Lines 89-95: Update node limit displays
- [ ] Lines 162-168: Update bandwidth limits in tree table
- [ ] Lines 368, 372-373: Update device plan displays

**Test 6.2:**
```javascript
// Browser test:
// 1. Open tree view
// 2. Check node bandwidth limits show decimals
// 3. Device plans should display fractional rates
```

### Task 6.3: Shaped Devices Page Updates
**File:** `rust/lqosd/src/node_manager/js_build/src/shaped-devices.js`

**Changes:**
- [ ] Line 32: Update plan display format
- [ ] Lines 224-228: Update live throughput displays

**Test 6.3:**
```javascript
// Browser test:
// 1. Open shaped devices page
// 2. Rate plans should show as "2.5 / 1.0 Mbps" format
// 3. Live updates should preserve decimals
```

### Task 6.4: Dashboard Component Updates
**Files:**
- `rust/lqosd/src/node_manager/js_build/src/dashlets/throughput_bps_dash.js`
- `rust/lqosd/src/node_manager/js_build/src/dashlets/top10flows_rate.js`
- `rust/lqosd/src/node_manager/js_build/src/dashlets/top_tree_summary.js`
- `rust/lqosd/src/node_manager/js_build/src/graphs/bits_gauge.js`

**Changes:**
- [ ] Update all rate displays to use updated formatting functions
- [ ] Ensure gauge calculations work with fractional rates
- [ ] Update tooltip and label formatting

**Test 6.4:**
```javascript
// Browser test:
// 1. Check all dashboard widgets
// 2. Gauges should scale correctly with fractional rates
// 3. Tooltips should show decimal precision
```

## Step 7: Integration Updates
**Estimated Time:** 2 hours
**Priority:** Medium

### Task 7.1: UISP Integration Updates
**File:** `rust/uisp_integration/src/strategies/full/shaped_devices_writer.rs`

**Changes:**
- [ ] Update lines 88-99 to preserve fractional rates
- [ ] Change `as u64` casting to maintain `f32` precision

**Test 7.1:**
```bash
cd rust/uisp_integration
cargo check
cargo test
# Integration should preserve fractional rates from UISP
```

### Task 7.2: Weight Calculation Updates
**File:** `rust/lqos_python/src/device_weights.rs`

**Changes:**
- [ ] Update line 79 to handle f32 rates properly
- [ ] Ensure weight calculations work with fractional rates

**Test 7.2:**
```bash
cd rust/lqos_python
cargo check
cargo test
# Weight calculations should handle fractional rates
```

## Step 8: CSV Format Updates
**Estimated Time:** 0.5 hours
**Priority:** Low

### Task 8.1: Update Example CSV
**File:** `ShapedDevices.example.csv`

**Changes:**
- [ ] Update header comments to mention decimal support
- [ ] Add example rows with fractional rates (0.5, 1.5, 2.5 Mbps)
- [ ] Update column descriptions

**Test 8.1:**
```bash
# Test example CSV loads correctly
cp ShapedDevices.example.csv ShapedDevices.csv
python3 LibreQoS.py --dry-run
# Should parse fractional examples without errors
```

## Step 9: End-to-End Testing
**Estimated Time:** 2-3 hours
**Priority:** Critical

### Task 9.1: Full System Test
**Test Cases:**
- [ ] **Small Fractional Rates (< 1 Mbps):**
  - CSV: 0.5 Mbps rates
  - TC commands use "kbit"
  - UI displays "0.5 Mbps"
  
- [ ] **Medium Fractional Rates (1-1000 Mbps):**
  - CSV: 2.5, 10.5, 100.5 Mbps rates  
  - TC commands use "mbit"
  - UI displays with decimals
  
- [ ] **Large Fractional Rates (> 1000 Mbps):**
  - CSV: 1500.5 Mbps rates
  - TC commands use "gbit"  
  - UI displays correctly

- [ ] **Backward Compatibility:**
  - Existing integer CSV files work unchanged
  - Integer rates display without unnecessary decimals
  - No performance degradation

- [ ] **Edge Cases:**
  - Very small rates (0.1 Mbps)
  - Rate validation works
  - Error handling for invalid inputs

**Test 9.1:**
```bash
# Complete workflow test
# 1. Edit ShapedDevices.csv with fractional rates
# 2. Run LibreQoS.py
# 3. Check generated TC commands
# 4. Start web interface  
# 5. Verify all UI components show fractional rates
# 6. Edit rates via web interface
# 7. Verify changes save correctly
```

## Completion Checklist

### Core Functionality
- [ ] Fractional rates can be entered in CSV
- [ ] Python parsing handles decimals correctly
- [ ] TC commands use appropriate units (kbit/mbit/gbit)
- [ ] Rust compilation works without errors
- [ ] UI accepts and displays fractional rates

### System Integration  
- [ ] LTS/Insight systems don't crash (temporary rounding)
- [ ] UISP integration preserves fractional rates
- [ ] Weight calculations work correctly
- [ ] All UI components display rates properly

### Testing & Quality
- [ ] Backward compatibility maintained
- [ ] Edge cases handled gracefully
- [ ] No performance degradation
- [ ] All automated tests pass
- [ ] Manual UI testing complete

### Documentation
- [ ] Example CSV updated
- [ ] Implementation plan documented
- [ ] Known limitations documented
- [ ] Future work identified

## Notes for Implementation

1. **Start with Step 1** - Core Rust changes block everything else
2. **Test after each step** - Don't accumulate compilation errors
3. **Use git branches** - Easy rollback if needed
4. **Manual UI testing** - Critical for user experience
5. **Document edge cases** - For future reference

## Future Work (Not in This Implementation)
- Full LTS database schema updates for fractional rates
- Complete Insight protocol support for decimals  
- Advanced rate unit parsing (e.g., "500k", "2.5m")
- Performance optimization for decimal calculations
- Enhanced validation and error messages