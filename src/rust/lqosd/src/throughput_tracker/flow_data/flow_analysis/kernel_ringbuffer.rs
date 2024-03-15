use std::{ffi::c_void, slice};
use zerocopy::FromBytes;
use lqos_sys::flowbee_data::FlowbeeKey;

#[repr(C)]
#[derive(FromBytes, Debug, Clone, PartialEq, Eq, Hash)]
pub struct FlowbeeEvent {
    key: FlowbeeKey,
    rtt: u64,
    effective_direction: u32,
}

#[no_mangle]
pub unsafe extern "C" fn flowbee_handle_events(
    _ctx: *mut c_void,
    data: *mut c_void,
    data_size: usize,
) -> i32 {
    println!("Event received");

    const EVENT_SIZE: usize = std::mem::size_of::<FlowbeeEvent>();
    if data_size < EVENT_SIZE {
        log::warn!("Warning: incoming data too small in Flowbee buffer");
        return 0;
    }

    let data_u8 = data as *const u8;
    let data_slice: &[u8] = slice::from_raw_parts(data_u8, EVENT_SIZE);
    if let Some(incoming) = FlowbeeEvent::read_from(data_slice) {
        println!("RTT: {}, Direction: {}", incoming.rtt, incoming.effective_direction);
    } else {
        log::error!("Failed to decode Flowbee Event");
    }

    /*const EVENT_SIZE: usize = std::mem::size_of::<HeimdallEvent>();
    if data_size < EVENT_SIZE {
        log::warn!("Warning: incoming data too small in Heimdall buffer");
        return 0;
    }

    //COLLECTED_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let data_u8 = data as *const u8;
    let data_slice: &[u8] = slice::from_raw_parts(data_u8, EVENT_SIZE);

    if let Some(incoming) = HeimdallEvent::read_from(data_slice) {
        store_on_timeline(incoming);
    } else {
        println!("Failed to decode");
    }*/

    0
}
