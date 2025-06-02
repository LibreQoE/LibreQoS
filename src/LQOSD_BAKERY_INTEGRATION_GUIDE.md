# LQOSD Bakery Integration Guide

## What needs to be added to lqosd

### 1. In `lqosd/Cargo.toml`
```toml
[dependencies]
lqos_bakery = { path = "../lqos_bakery" }
```

### 2. In `lqosd/src/main.rs`

After XDP initialization but before starting the bus server:

```rust
// Start the bakery thread
let bakery_sender = lqos_bakery::start_bakery();
info!("Bakery thread started");
```

### 3. In the bus handler (where BusRequests are processed)

Pass the bakery_sender to the handler and add cases for bakery commands:

```rust
match request {
    // ... existing cases ...
    
    BusRequest::BakeryClearPriorSettings => {
        bakery_sender.send(BakeryCommands::ClearPriorSettings)?;
        BusResponse::Ack
    },
    
    BusRequest::BakeryMqSetup => {
        bakery_sender.send(BakeryCommands::MqSetup)?;
        BusResponse::Ack
    },
    
    BusRequest::BakeryAddStructuralHTBClass { 
        interface, parent, classid, rate_mbps, ceil_mbps, site_hash, r2q 
    } => {
        bakery_sender.send(BakeryCommands::AddStructuralHTBClass {
            interface, parent, classid, rate_mbps, ceil_mbps, site_hash, r2q
        })?;
        BusResponse::Ack
    },
    
    BusRequest::BakeryAddCircuitHTBClass {
        interface, parent, classid, rate_mbps, ceil_mbps, circuit_hash, comment, r2q
    } => {
        bakery_sender.send(BakeryCommands::AddCircuitHTBClass {
            interface, parent, classid, rate_mbps, ceil_mbps, circuit_hash, comment, r2q
        })?;
        BusResponse::Ack
    },
    
    BusRequest::BakeryAddCircuitQdisc {
        interface, parent_major, parent_minor, circuit_hash, sqm_params
    } => {
        bakery_sender.send(BakeryCommands::AddCircuitQdisc {
            interface, parent_major, parent_minor, circuit_hash, sqm_params
        })?;
        BusResponse::Ack
    },
    
    BusRequest::BakeryExecuteTCCommands { commands, force_mode } => {
        bakery_sender.send(BakeryCommands::ExecuteTCCommands { commands, force_mode })?;
        BusResponse::Ack
    },
}
```

### 4. Error Handling

The bakery sender should be stored in a way that's accessible to the bus handler. Common patterns:

1. **Pass as parameter**: Thread the bakery_sender through to where bus requests are handled
2. **Global state**: Store in a static or shared state structure
3. **Actor pattern**: Include in the bus handler's state

### 5. Verification

Once integrated, the bakery should:
- Start its own thread via `start_bakery()`
- Receive commands via the channel
- Write to `tc-rust.txt` when `WRITE_TC_TO_FILE = true`
- Execute TC commands when `WRITE_TC_TO_FILE = false`

## Testing

1. Start lqosd with bakery integration
2. Run `python3 diagnose_bakery.py` - should create `tc-rust.txt`
3. Run `python3 test_bakery_integration.py` - should show matching outputs