# LQOS Bakery - Phase 1 Implementation Plan

## Overview
Phase 1 goal: Mirror the existing LibreQoS.py TC (Traffic Control) functionality in Rust, maintaining exact behavioral compatibility while keeping all logic in Python.

## Immediate Tasks

### 1. Verify and Update Helper Functions ✅
- [x] Review `r2q_bandwidth()` function - verify it matches Python's `r2q()`
  - **Finding**: Python's `calculateR2q()` IS still used (line 883 in LibreQoS.py)
  - **Issue**: Python sets a global R2Q variable used by quantum(), Rust calculates but doesn't use it
  - **Fixed**: Renamed to `calculate_r2q()`, uses floating-point division to match Python exactly
- [x] Review `quantum()` function - verify it matches Python's `quantum()`
  - **Bug Found**: Rust hardcoded division by 8, Python uses dynamically calculated R2Q
  - **Fixed**: Now accepts r2q as parameter and matches Python behavior
- [x] Update both functions to support fractional speeds (building on previous fractional speeds work)
  - Both functions now use `f64` for rates
- [x] **Create unit tests** to verify Rust matches Python exactly:
  - Test `calculate_r2q()` with various bandwidth values (1, 10, 100, 1000, 10000 Mbps)
  - Test `quantum()` with the same range of values
  - Include fractional speed tests (1.5, 10.5, 999.9 Mbps)
  - All tests pass with Python-calculated expected values

### 2. Fractional Speed Formatting ✅
- [x] Create helper function to format bandwidth values with appropriate units (Mbit, Kbit, etc.)
- [x] Match Python's formatting behavior exactly (see `format_rate_for_tc()` at line 81)
  - Rates ≥ 1000 Mbps → "X.Xgbit" (1 decimal place)
  - Rates ≥ 1 Mbps → "X.Xmbit" (1 decimal place)
  - Rates < 1 Mbps → "Xkbit" (0 decimal places)
- [x] Support fractional values (e.g., 1.5Mbit, 500.5Kbit)
- [x] Create unit tests for format_rate_for_tc() with edge cases
  - All tests pass with exact string matches

### 3. TC Command Mirroring ✅
- [x] Identify all locations in LibreQoS.py where TC commands are executed
- [x] Create Rust equivalents for each TC operation type:
  - [x] Queue creation (qdisc add/replace/delete)
    - `replace_mq()`, `make_top_htb()`, `delete_root_qdisc()`
  - [x] Class creation (class add/change/delete)
    - `add_htb_class()`, `add_circuit_htb_class()`, `add_node_htb_class()`
    - `make_parent_class()`, `make_default_class()`
  - [ ] Filter operations (not used in current LibreQoS.py)
  - [x] Queue statistics queries
    - `has_mq_qdisc()` / `is_mq_installed()`
- [x] Ensure all TC command builders support fractional speeds
  - All functions use `f64` for bandwidth parameters
- [x] Add `circuit_hash: i64` parameter to circuit functions for future phases
  - Forward-thinking design for Phase 2+ tracking

### 4. Centralized TC Command Execution ✅
- [x] Create a single function to handle all TC command execution
  - Implemented `execute_tc_command()` in `tc_control.rs`
- [x] Add compile-time constant `WRITE_TC_TO_FILE` to control behavior
- [x] When `WRITE_TC_TO_FILE` is true:
  - Append commands to `tc-rust.txt` (similar to Python's `linux_tc.txt`)
  - Format: Just the TC arguments (no `/sbin/tc` prefix), one per line
  - Match Python's format exactly for easy comparison
- [x] When `WRITE_TC_TO_FILE` is false:
  - Execute commands via `std::process::Command`
  - Handle errors appropriately
- [x] Update all existing TC command calls to use this function
  - All TC functions now use centralized execution
- [ ] Consider batching commands like Python does (using `tc -b` with file)
  - Future enhancement
- [x] This enables comparison testing between Python and Rust outputs

### 5. Python Integration Points ✅
- [x] Add comments to LibreQoS.py indicating future Rust API call locations
- [x] Document the expected API interface for each call
- [x] Do NOT implement actual calls - just mark the locations

#### Integration Points Identified:

1. **ClearPriorSettings** (Line 134)
   - Called before setting up new queues
   - Data: interfaceA, interfaceB, on_a_stick flag
   - Rust: `BakeryCommands::ClearPriorSettings`

2. **MqSetup** (Line 885)
   - Creates MQ root + HTB hierarchy with defaults
   - Data: interface config, bandwidth caps, queuesAvailable, default_sqm
   - Rust: `BakeryCommands::MqSetup`

3. **AddStructuralHTBClass** (Line 966)
   - Creates HTB classes for sites/APs (no qdisc)
   - Data: interface, parent, classid, rate/ceil, site_hash, r2q
   - Rust: `add_structural_htb_class()`

4. **AddCircuitHTBClass + AddCircuitQdisc** (Line 995)
   - Creates HTB class + CAKE qdisc for circuits
   - Data: interface, parent, classid, rate/ceil, circuit_hash, sqm_params, r2q
   - Rust: `add_circuit_htb_class()` + `add_circuit_qdisc()`

5. **ExecuteTCCommands** (Line 1099) - Optional bulk approach
   - Execute all commands at once via `tc -b`
   - Data: list of all TC commands, debug flag
   - Could be alternative to individual calls

## TC Command Categories to Mirror

### Queue Management ✅
- MQ (Multi-Queue) setup ✅ (complete)
- HTB hierarchy creation ✅ (complete)
- CAKE qdisc configuration ✅ (via SQM parameters)
- FQ-CoDel qdisc configuration ✅ (via SQM parameters)

### Class Management ✅
- Root classes ✅ (make_parent_class)
- Structural classes ✅ (add_structural_htb_class) - Sites/APs from network.json
  - Accept `site_hash: i64` for future tracking
  - Only HTB classes, NO qdiscs (intermediate hierarchy nodes)
- Circuit classes ✅ (add_circuit_htb_class) - Customer circuits
  - Accept `circuit_hash: i64` for future tracking
  - Both HTB class AND CAKE/fq_codel qdisc (leaf nodes)
- Default classes ✅ (make_default_class)
- Class modifications ❌ (not needed for Phase 1 - full tree rebuild)

### Additional Operations ✅
- Queue deletion/cleanup ✅ (delete_root_qdisc, clear_all_queues)
- Statistics gathering ✅ (has_mq_qdisc)
- Error handling patterns ✅ (via Result types)
- SQM fixup for low bandwidth ✅ (sqm_fixup_rate with fractional support)

## Progress Summary

### Completed ✅
1. Centralized TC command execution with file logging option
2. Fixed r2q/quantum calculations to match Python exactly
3. Implemented fractional speed formatting (gbit/mbit/kbit)
4. Created comprehensive unit tests verifying Python compatibility
5. Updated all existing TC functions to support fractional speeds
6. MQ setup and HTB hierarchy creation for default queues
7. Circuit-specific queue creation with `circuit_hash` parameter
8. Structural node queue creation with `site_hash` parameter
9. Clear API distinction between structural (HTB only) and circuit (HTB+qdisc) queues
10. SQM fixup function with fractional speed support
11. Complete TC command mirroring for all Phase 1 operations

### Still To Do
1. ~~Python integration points (API comments in LibreQoS.py)~~ ✅
2. Integration testing comparing Python vs Rust TC output
3. Documentation for using the Bakery in production
4. Actual implementation of Python->Rust calls (Phase 1 deployment)

### Implementation Notes
- Clear distinction between structural nodes (sites/APs) and circuits:
  - **Structural nodes**: Only HTB classes, tracked by `site_hash: i64`
  - **Circuits**: HTB class + CAKE qdisc, tracked by `circuit_hash: i64`
- Added `circuit_hash` and `site_hash` parameters for future Phase 2+ tracking
- `sqm_fixup_rate()` now uses ranges to handle fractional speeds correctly
- All bandwidth parameters use `f64` throughout for consistent fractional support
- TC commands can be logged to `tc-rust.txt` by setting `WRITE_TC_TO_FILE = true`
- Comprehensive module documentation in `tc_control.rs` explains the architecture

## Design Principles
1. **Exact Behavioral Match**: Every TC command generated by Rust must match Python's output exactly
2. **No Logic in Rust**: All decision-making remains in Python; Rust only executes TC commands
3. **Fractional Speed Support**: All bandwidth specifications must support fractional values
4. **Error Handling**: Mirror Python's error handling behavior

## Next Steps After Phase 1
- Phase 2: Lazy queue creation (only create when traffic detected)
- Phase 3: Differential updates (track changes, apply only deltas)
- Phase 4: Live migration (lossless queue movement)

## Python Bugs Found During Analysis

### 1. **`sqmFixupRate()` function incompatible with fractional speeds** (line 927)
   - Type hint still expects `int` for rate parameter
   - `match` statement uses exact equality, won't match fractional rates like 1.5
   - CAKE RTT adjustments for low bandwidth won't apply to fractional speed circuits
   - Needs to be updated to handle rate ranges instead of exact matches
   - **Rust implementation**: Fixed in `sqm_fixup_rate()` using ranges (≤1.5, ≤2.5, etc.)

### 2. **Integer conversions losing fractional precision**
   - **Lines 397-400**: Circuit bandwidth comparisons convert to `int()`, losing fractional parts
   - **Lines 642-645**: `inheritBandwidthMaxes()` converts all bandwidth values to integers
   - These conversions will cause fractional speeds to be truncated

### 3. **Exact equality checks on float values**
   - **Lines 960-963**: Checks like `if min_down == 1:` may fail with floating-point values
   - Should use approximate comparisons or ranges

### 4. **Rounding without decimal specification**
   - **Lines 894, 915**: `round()` without decimals parameter rounds to nearest integer
   - Should specify decimal places to preserve fractional values where needed

### 5. **Good news**: Most of the codebase already handles fractional speeds correctly
   - CSV parsing uses `float()` appropriately
   - `format_rate_for_tc()` handles fractional formatting well
   - Most mathematical operations preserve float types

## Notes
- The Python team (Robert and Frank) maintains control over business logic
- Rust side focuses purely on efficient TC command execution
- This separation allows fast iteration on logic while gaining performance benefits