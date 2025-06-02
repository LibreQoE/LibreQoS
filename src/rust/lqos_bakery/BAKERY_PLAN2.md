# LQOS Bakery - Phase 2 Implementation Plan: Lazy Queue Creation

## Overview
Phase 2 goal: Implement lazy queue creation where circuit queues are only created when traffic is detected, with automatic expiration of idle queues. This builds on Phase 1's foundation to provide dynamic queue management.

## Implementation Strategy

The lazy queue system operates on the principle of "create on demand, expire on idle." Structural queues (sites/APs) are built immediately since they form the network hierarchy, while circuit queues are stored as metadata and only created when traffic flows through them.

## 🚀 Current Implementation Status

### ✅ **COMPLETED (Steps 1-5, 7-8)**
- **Configuration Extension**: Added `lazy_queues` and `lazy_expire_seconds` fields with backward compatibility
- **Data Structures**: Complete `CircuitQueueInfo`, `StructuralQueueInfo`, and `BakeryState` implementation
- **Enhanced Commands**: `UpdateCircuit` and `CreateCircuit` commands implemented
- **Queue State Management**: Full lazy creation logic (Phase A/B/C) with backward compatibility
- **Throughput Tracker Integration**: Connected bakery to traffic detection (Step 5)
  - UpdateCircuit integrated at line 171 for existing traffic
  - CreateCircuit integrated at line 223 for new circuits
- **Lazy Queue Control Logic**: Proper flag checking and behavior switching
- **Thread Safety**: Single master lock, duplicate prevention, short critical sections
- **ExecuteTCCommands Bulk Handler**: Modified to support lazy queues with TC command parsing
- **Hash Function Consistency**: Fixed circuit hash mismatch by exposing Rust `hash_to_i64` to Python

### 🔄 **IN PROGRESS** 
- **Thread Safety**: Update batching and pruning coordination (basic safety implemented)

### ⏳ **TODO (Steps 6, 9-12)**
- **Queue Pruning System**: Background thread for expiration (Step 6)  
- **Comprehensive Testing**: Full automated test suite (Step 9)
- **Web UI Configuration**: HTML/JavaScript form integration (Step 10)
- **Manual Testing**: System-level validation (Step 11)
- **Documentation**: Comprehensive docs and logging (Step 12)

### 🧪 **MANUAL TESTING RESULTS**
- ✅ lqosd compiles successfully
- ✅ lqosd loads without errors  
- ✅ Backward compatibility with lazy_queues=false fixed (force_mode logic corrected)
- ✅ TC bulk command execution working correctly
- ✅ Queues created successfully with both lazy and non-lazy modes
- ✅ **Lazy queue creation verified working** - circuit queues created on traffic detection
- ✅ **Hash consistency fixed** - Python and Rust use same `hash_to_i64` function
- ✅ **Fractional bandwidth rates working** - 0.3/1.0 Mbps rates handled correctly
- ⏳ Web UI configuration not yet available (expected - Step 10)

## ⚠️ Critical Pitfalls to Avoid

### 1. Duplicate Queue Creation Race
**Problem**: Circuits with multiple devices can trigger multiple CreateCircuit commands in one cycle for the same circuit_hash.
**Solution**: Always check `circuit.created` flag before creating. Use idempotent creation logic.

### 2. Update Command Flooding  
**Problem**: High traffic circuits generate many UpdateCircuit commands, overwhelming the bakery.
**Solution**: Batch updates in a `HashSet<i64>` per cycle to deduplicate circuit_hash values. Process batches atomically.

### 3. Create/Update/Remove Race Conditions
**Problem**: Main bakery thread (create/update) vs pruning thread (remove) can cause logical races that are extremely difficult to debug.
**Solution**: Single master lock `Arc<Mutex<BakeryState>>` for ALL operations. Never interleave create/update/remove. Process commands in strict order: creates first, then updates, then (separately) removals.

## Implementation Order

### 1. Configuration Extension ✅
**File:** `rust/lqos_config/src/etc/v15/queues.rs`
- [x] Add `lazy_queues: Option<bool>` field (default: None for backward compatibility)
- [x] Add `lazy_expire_seconds: Option<u64>` field (default: Some(600) = 10 minutes)
- [x] Update serialization and validation logic

### 2. Circuit Queue Data Structures ✅
**File:** `rust/lqos_bakery/src/lib.rs` (new structures)
- [x] Create `CircuitQueueInfo` struct to store all queue creation parameters:
  - Interface, parent, classid, rate/ceil, circuit_hash, sqm_params, r2q
  - `last_updated: u64` timestamp (using `std::time`)
  - `created: bool` flag to track if queue is actually built (prevents duplicates)
- [x] Create `StructuralQueueInfo` struct for hierarchy queues:
  - Interface, parent, classid, rate/ceil, site_hash, r2q
- [x] Create unified state container (CRITICAL for race prevention):
  ```rust
  struct BakeryState {
      circuits: HashMap<i64, CircuitQueueInfo>,
      structural: HashMap<i64, StructuralQueueInfo>,
      pending_updates: HashSet<i64>,  // Batch and deduplicate updates
      pending_creates: HashSet<i64>,  // Batch and deduplicate creates
  }
  ```
- [x] Wrap in single master lock: `Arc<Mutex<BakeryState>>` for ALL operations

### 3. Enhanced Bakery Commands ✅
**File:** `rust/lqos_bakery/src/lib.rs`
- [x] Add `BakeryCommands::UpdateCircuit { circuit_hash: i64 }` for "still alive" signals
- [x] Add `BakeryCommands::CreateCircuit { circuit_hash: i64 }` for explicit creation requests
- [x] Modify existing commands to work with the new data structures

### 4. Queue State Management ✅
**File:** `rust/lqos_bakery/src/lib.rs` (bakery thread logic)
- [x] **Phase A: Structural Queues First**
  - When `AddStructuralHTBClass` received: create immediately AND store in structural data
  - Build the complete hierarchy before any circuit queues
- [x] **Phase B: Circuit Storage**
  - When `AddCircuitHTBClass`/`AddCircuitQdisc` received: store in circuit data structure, do NOT create
  - Set `created: false` and `last_updated: 0`
- [x] **Phase C: Lazy Creation**
  - When `CreateCircuit` received: check if circuit exists in storage, create if not already created
  - When `UpdateCircuit` received: update `last_updated` timestamp, create if needed
- [x] **Backward Compatibility**: When `lazy_queues = false`, create circuit queues immediately (Phase 1 behavior)

### 5. Throughput Tracker Integration ✅
**File:** `rust/lqosd/src/throughput_tracker/tracking_data.rs`
- [x] Clone bakery sender and pass to tracking functions (like other channels)
- [x] **Line 171 integration**: When packets change (existing traffic):
  ```rust
  // Call to Bakery Update Goes Here.
  if let Some(bakery_sender) = &bakery_sender {
      let _ = bakery_sender.send(BakeryCommands::UpdateCircuit { 
          circuit_hash 
      }).map_err(|e| warn!("Failed to send UpdateCircuit to bakery: {}", e));
  }
  ```
- [x] **Line 223 integration**: When new circuit detected:
  ```rust
  // Call to Bakery Queue Creation Goes Here.
  if let Some(bakery_sender) = &bakery_sender {
      let _ = bakery_sender.send(BakeryCommands::CreateCircuit { 
          circuit_hash 
      }).map_err(|e| warn!("Failed to send CreateCircuit to bakery: {}", e));
  }
  ```

### 6. Queue Pruning System
**File:** `rust/lqos_bakery/src/lib.rs` (new background thread)
- [ ] Create separate pruning thread spawned by bakery
- [ ] Read `lazy_expire_seconds` from config
- [ ] If expiration disabled (None), don't start pruning thread
- [ ] Periodic check (every 30 seconds) for circuits older than threshold:
  - Compare `last_updated` against current time
  - Remove expired queues using TC delete commands
  - Remove from circuit data structure
  - Log pruning actions for debugging

### 7. Lazy Queue Control Logic ✅
**File:** `rust/lqos_bakery/src/lib.rs` (bakery thread main loop)
- [x] Check `lazy_queues` config flag in all circuit operations (via `is_lazy_queues_enabled()`)
- [x] **If lazy_queues = false**: behave exactly like Phase 1 (immediate creation)
- [x] **If lazy_queues = true**: use lazy creation logic
- [x] Ensure backward compatibility - existing deployments work unchanged

### 8. Thread Safety and Critical Race Condition Prevention ✅ (Partial)
**File:** `rust/lqos_bakery/src/lib.rs`

#### 8.1 Locking Strategy (CRITICAL) ✅
- [x] **Single Master Lock**: Use one `Arc<Mutex<BakeryState>>` containing all shared data
  ```rust
  struct BakeryState {
      circuits: HashMap<i64, CircuitQueueInfo>,
      structural: HashMap<i64, StructuralQueueInfo>,
      pending_updates: HashSet<i64>,  // Batch update commands
      pending_creates: HashSet<i64>,  // Batch create commands
  }
  ```
- [x] **Lock Order Enforcement**: Always acquire locks in same order to prevent deadlocks
- [x] **Short Critical Sections**: Minimize time holding locks (release before TC commands)
- [x] **Atomic Operations**: Never interleave create/update/remove operations

#### 8.2 Race Condition Prevention ✅
- [x] **Duplicate Creation Protection**:
  ```rust
  // Before creating any queue, check:
  if circuit_info.created {
      return Ok(()); // Already created, skip
  }
  ```
- [ ] **Update Batching**: Collect multiple UpdateCircuit calls per cycle (TODO: Not yet implemented)
- [ ] **Pruning Coordination**: Ensure pruning thread cannot remove while creating (TODO: Pruning not implemented yet)

#### 8.3 Synchronization Patterns 🔄 (Partial)
- [x] **Basic Command Processing**: Each command handled individually with proper locking
- [ ] **Batched Processing**: Batch commands per cycle (TODO: Future optimization)
- [x] **Graceful Error Handling**: Handle lock poisoning with `map_err()` patterns
- [ ] **Pruning Thread Coordination**: (TODO: Pruning thread not implemented yet)

### 9. Automated Testing Implementation
**Files:** `rust/lqos_bakery/src/lib.rs` + `tests/` directory

#### 9.1 Unit Tests (Automated)
- [ ] **Data Structure Tests** (`tests/circuit_queue_info_tests.rs`):
  ```rust
  #[test] fn test_circuit_queue_info_creation()
  #[test] fn test_circuit_queue_info_serialization()
  #[test] fn test_last_updated_timestamp()
  #[test] fn test_created_flag_toggle()
  ```
- [ ] **HashMap Operations** (`tests/queue_storage_tests.rs`):
  ```rust
  #[test] fn test_circuit_storage_insert_retrieve()
  #[test] fn test_structural_storage_operations()
  #[test] fn test_concurrent_access_simulation()
  ```
- [ ] **Configuration Logic** (`tests/config_tests.rs`):
  ```rust
  #[test] fn test_lazy_queues_disabled_behavior()
  #[test] fn test_lazy_queues_enabled_behavior()
  #[test] fn test_expiration_time_validation()
  #[test] fn test_optional_expiration_handling()
  ```

#### 9.2 Integration Tests (Automated)
- [ ] **Command Flow Tests** (`tests/integration_tests.rs`):
  ```rust
  #[test] fn test_structural_then_circuit_creation_order()
  #[test] fn test_update_circuit_before_creation()
  #[test] fn test_create_circuit_idempotency()
  #[test] fn test_command_routing_from_bus()
  ```
- [ ] **Expiration Logic** (`tests/expiration_tests.rs`):
  ```rust
  #[test] fn test_expiration_with_controlled_time()
  #[test] fn test_no_expiration_when_disabled()
  #[test] fn test_expiration_thread_startup_shutdown()
  #[test] fn test_recent_activity_prevents_expiration()
  ```

#### 9.3 Mock-Based Testing (Automated)
- [ ] **TC Command Validation** (`tests/tc_command_tests.rs`):
  ```rust
  #[test] fn test_tc_commands_not_executed_when_stored()
  #[test] fn test_tc_commands_executed_on_creation()
  #[test] fn test_tc_delete_commands_on_expiration()
  ```
- [ ] **Thread Safety & Race Conditions** (`tests/concurrency_tests.rs`):
  ```rust
  #[test] fn test_concurrent_circuit_updates()
  #[test] fn test_update_while_pruning()
  #[test] fn test_multiple_create_requests_same_circuit()
  #[test] fn test_duplicate_creation_prevention()
  #[test] fn test_update_batching_deduplication()
  #[test] fn test_create_update_remove_coordination()
  #[test] fn test_poisoned_mutex_recovery()
  ```

#### 9.4 Backward Compatibility (Automated)
- [ ] **Phase 1 Behavior Preservation** (`tests/compatibility_tests.rs`):
  ```rust
  #[test] fn test_immediate_creation_when_lazy_disabled()
  #[test] fn test_existing_commands_unchanged()
  #[test] fn test_config_migration_compatibility()
  ```

### 10. Web UI Configuration Integration
**Files:** Web UI configuration system
- [ ] **HTML Form (`config_queues.html`)**:
  - Add checkbox for `lazy_queues` with descriptive label
  - Add number input for `lazy_expire_seconds` with min=30 validation
  - Follow existing Bootstrap 5 form patterns
- [ ] **JavaScript (`config_queues.js`)**:
  - Add form field population in `loadConfig()` callback
  - Add validation in `validateConfig()` (min 30 seconds for expiration)
  - Add field mapping in `updateConfig()` (camelCase HTML → snake_case config)
  - Handle optional values (null for default expiration)
- [ ] **API Integration (`config.rs`)**:
  - No changes needed - existing endpoints handle new config fields automatically
  - Fields automatically serialized/deserialized through existing Config struct

### 11. Manual Integration Testing
**Requirements:** Full system setup with root access

#### 11.1 Build Process Testing (Manual)
- [ ] **JavaScript Compilation Verification**:
  ```bash
  cd /path/to/lqosd
  ./copy_files.sh
  # Verify: No esbuild compilation errors
  # Verify: New config fields present in generated JavaScript
  ```
- [ ] **Rust Compilation**:
  ```bash
  cargo build --release
  # Verify: All new bakery features compile without warnings
  # Verify: Configuration changes integrate properly
  ```

#### 11.2 Configuration Testing (Manual)
- [ ] **Web UI Configuration Flow**:
  1. Start lqosd with root privileges
  2. Navigate to queue configuration page
  3. Verify new fields appear correctly
  4. Test form validation (invalid expiration times)
  5. Save configuration and verify persistence
  6. Restart lqosd and verify settings loaded correctly

#### 11.3 Functional Testing (Manual)
- [ ] **Lazy Queue Disabled (Backward Compatibility)**:
  1. Set `lazy_queues = false` in config
  2. Run LibreQoS.py queue generation
  3. Verify all circuit queues created immediately (Phase 1 behavior)
  4. Compare TC command output with Phase 1 baseline

- [ ] **Lazy Queue Enabled**:
  1. Set `lazy_queues = true`, `lazy_expire_seconds = 60`
  2. Run LibreQoS.py to create structural queues only
  3. Verify circuit queues NOT created in TC
  4. Generate traffic for specific circuits
  5. Verify those circuit queues get created
  6. Wait for expiration timeout
  7. Verify unused queues get removed

#### 11.4 Traffic Simulation Testing (Manual)
- [ ] **Create Test Traffic Scenarios**:
  ```bash
  # Script to generate controlled traffic patterns
  #!/bin/bash
  # Generate traffic to circuit A for 30 seconds
  iperf3 -c <circuit_A_ip> -t 30 &
  
  # Generate traffic to circuit B for 10 seconds, then stop
  iperf3 -c <circuit_B_ip> -t 10 &
  
  # Monitor queue creation/deletion
  watch -n 5 "tc -s qdisc show dev <interface>"
  ```

#### 11.5 Race Condition & Edge Case Testing (Manual)
- [ ] **Duplicate Creation Prevention**:
  1. Create circuit with multiple devices (same circuit_hash)
  2. Generate simultaneous traffic to all devices
  3. Verify only one queue created per circuit_hash
  4. Monitor logs for duplicate creation attempts

- [ ] **Update Batching Efficiency**:
  1. Generate high-frequency traffic to many circuits
  2. Monitor bakery command queue depth
  3. Verify updates are batched and deduplicated
  4. Check that update processing doesn't overwhelm system

- [ ] **Create/Update/Remove Coordination**:
  1. Set very short expiration time (30 seconds)
  2. Generate intermittent traffic patterns
  3. Force rapid create→update→expire cycles
  4. Verify no race conditions between operations
  5. Monitor for "queue not found" or "already exists" errors

#### 11.6 Stress Testing (Manual)
- [ ] **High Traffic Load**:
  1. Configure short expiration time (30 seconds)
  2. Generate intermittent traffic to many circuits
  3. Monitor system resources during queue churn
  4. Verify no memory leaks or performance degradation
  5. Check log files for errors or race conditions
  6. Test with 1000+ concurrent circuits

#### 11.7 Automated Test Runner Integration
- [ ] **Continuous Testing Setup**:
  ```bash
  # Add to CI/CD pipeline
  cd rust/lqos_bakery
  cargo test --all-features
  cargo test --release
  
  # Integration with existing test suite
  cd ../..
  python3 run_tests.py --include-bakery-phase2
  ```

### 12. Documentation and Logging
**File:** Various
- [ ] Add comprehensive documentation for new structs and functions
- [ ] Add debug logging for:
  - Circuit queue storage operations
  - Lazy creation events
  - Expiration events
  - Configuration flag effects
  - Web UI configuration updates
- [ ] Update README with Phase 2 capabilities and web UI changes

## Data Flow Architecture

### Lazy Queue Creation Flow:
1. **Structural Setup**: LibreQoS.py creates structural queues → Bakery creates immediately
2. **Circuit Registration**: LibreQoS.py registers circuits → Bakery stores metadata only
3. **Traffic Detection**: lqosd detects new traffic → Sends CreateCircuit to Bakery
4. **Queue Creation**: Bakery checks storage → Creates actual TC queue if not exists
5. **Traffic Updates**: Ongoing traffic → Sends UpdateCircuit to keep queue alive
6. **Expiration**: No traffic for threshold → Pruning thread removes queue

### Thread Architecture:
- **Main Bakery Thread**: Handles commands, manages data structures
- **Pruning Thread**: Periodic cleanup of expired queues (optional)
- **lqosd Throughput Thread**: Sends Update/Create commands to Bakery

## Configuration Integration

### Example Configuration:
```toml
[queues]
lazy_queues = true
lazy_expire_seconds = 600  # 10 minutes, omit for no expiration
```

### Web UI Integration Patterns:
Based on analysis of existing queue configuration patterns:

**HTML Structure (config_queues.html):**
```html
<div class="mb-3 form-check">
    <input type="checkbox" class="form-check-input" id="lazyQueues">
    <label class="form-check-label" for="lazyQueues">Enable Lazy Queues</label>
    <div class="form-text">Enable dynamic queue creation for fractional rate plans</div>
</div>

<div class="mb-3">
    <label for="lazyExpireSeconds" class="form-label">Lazy Queue Expiration (seconds)</label>
    <input type="number" class="form-control" id="lazyExpireSeconds" min="30">
    <div class="form-text">Time before unused lazy queues are removed (leave blank for default)</div>
</div>
```

**JavaScript Integration (config_queues.js):**
- Form population: `document.getElementById("lazyQueues").checked = queues.lazy_queues ?? false`
- Validation: Minimum 30 seconds for expiration time
- Update mapping: `lazy_queues: document.getElementById("lazyQueues").checked`
- Optional handling: `lazy_expire_seconds: value ? parseInt(value) : null`

**Build Process:**
- Use `copy_files.sh` in lqosd directory to verify JavaScript compilation
- Requires full lqosd build and execution (root access needed) for testing

### Backward Compatibility:
- Default `lazy_queues = false` maintains Phase 1 behavior
- Existing configurations work without changes
- No performance impact when lazy queues disabled
- Existing API endpoints automatically handle new config fields

## Key Design Principles

1. **Structural First**: Always build the hierarchy before any circuit queues
2. **Store Then Create**: All circuit info stored immediately, queues created on-demand
3. **Thread Safe**: All shared data protected by mutexes
4. **Configurable**: Can be completely disabled for traditional behavior
5. **Observable**: Comprehensive logging for debugging and monitoring
6. **Resilient**: Handle edge cases gracefully (missing circuits, expired data, etc.)

## Success Criteria

### Functional Requirements
- [ ] Structural queues created immediately as before
- [ ] Circuit queues only created when traffic flows
- [ ] Idle circuits automatically pruned after configured timeout
- [ ] Zero performance impact when lazy_queues = false
- [ ] Thread-safe operation under concurrent load
- [ ] Web UI configuration working correctly

### Testing Requirements
- [ ] **Automated Test Coverage ≥ 90%**:
  - Unit tests for all data structures and core logic
  - Integration tests for command flows and timing
  - Concurrency tests for thread safety
  - Backward compatibility verification
- [ ] **Manual Testing Scenarios Passed**:
  - Web UI configuration flow complete
  - Traffic simulation creates/removes queues correctly
  - Stress testing shows no performance degradation
  - Build process verification successful
- [ ] **Performance Benchmarks**:
  - Queue creation latency < 10ms
  - Memory usage stable during queue churn
  - No impact on existing functionality when disabled

### Quality Assurance
- [ ] Clear documentation and comprehensive logging
- [ ] All tests passing in CI/CD pipeline
- [ ] Code review completed for thread safety
- [ ] Configuration validation working properly

## Future Enhancements (Beyond Phase 2)

- Queue priority system (create high-bandwidth circuits first)
- Adaptive expiration times based on traffic patterns
- Queue statistics and analytics
- Hot queue migration for load balancing

## 📚 Lessons Learned & Critical Fixes

### 1. ExecuteTCCommands Bulk Handler Implementation
**Problem**: LibreQoS.py uses bulk TC commands which bypassed lazy queue logic entirely.
**Solution**: Implemented TC command parsing to:
- Identify circuit vs structural commands
- Route circuit commands through lazy handlers when lazy_queues enabled
- Execute structural commands immediately

### 2. Circuit Hash Integrity
**Critical Issue**: Circuit hash must ALWAYS be derived from circuit ID in ShapedDevices.csv, NEVER calculated from classid or other sources.
**Solution**: 
- Removed dangerous `generate_circuit_hash_from_classid()` function
- Added tests to ensure circuit_hash only comes from:
  - BakeryCommands (passed by LibreQoS.py)
  - TC command comments (parsed, not calculated)

### 3. Force Mode Logic Fix
**Problem**: TC bulk execution failed with "Operation not permitted" when lazy_queues=false
**Root Cause**: Force mode logic was inverted: `force_mode = logging.DEBUG > logging.root.level`
**Fix**: Corrected to `force_mode = logging.root.level > logging.DEBUG`
**Impact**: This ensures `-f` flag is used for TC bulk execution, allowing queues to be created even when some already exist

### 4. Backward Compatibility Testing
**Key Insight**: Must test both scenarios:
- Fresh boot (no queues exist) - uses `qdisc replace` + `add` commands
- Reload (queues exist) - relies on `-f` flag to ignore duplicate errors
**Verification**: Both scenarios now work correctly with the force mode fix

### 5. TC Command Format for Bulk Files
**Important**: TC bulk files should NOT include the `/sbin/tc` prefix
**Correct**: `qdisc add dev eth0...`
**Incorrect**: `/sbin/tc qdisc add dev eth0...`

### 6. Integration Challenges
**Finding**: Integrating with existing Python code requires careful attention to:
- Command format differences
- Execution permissions
- Error handling and reporting
- Maintaining exact backward compatibility

### 7. TC Bulk Execution with Existing Queues
**Problem**: When queues exist from a previous run, TC `add` commands fail but `-f` flag hides errors
**Symptoms**: 
- TC bulk execution returns status 0 (success)
- Empty stdout/stderr due to `-f` flag
- But queues aren't actually created
**Root Cause**: The `-f` flag suppresses errors but doesn't fix them - `add` still fails on existing objects
**Solution**: Clear prior settings before bulk execution when `qdisc replace` is detected (indicates full reload)

### 8. Hash Function Consistency Between Python and Rust
**Problem**: Circuit hash mismatch prevented lazy queue creation
**Symptoms**:
- Python calculated different hash than Rust for same circuit ID
- Bakery stored circuits but couldn't match them when traffic arrived
- Example: Python hash `-4663096281490929287` vs Rust hash `-8456361809408299971`
**Root Cause**: Python used SHA256 while Rust used DefaultHasher
**Solution**: 
- Exposed Rust's `hash_to_i64` function to Python via `lqos_python` module
- Updated LibreQoS.py to use the Rust hash function exclusively
- Ensures both sides use identical hashing algorithm
**Impact**: Circuit hashes now match perfectly, enabling lazy queue creation to work