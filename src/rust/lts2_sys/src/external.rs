use std::ffi::{c_char, c_int};
use crate::shared_types::{CircuitCakeDrops, CircuitCakeMarks, CircuitRetransmits, CircuitRtt, CircuitThroughput, SiteCakeDrops, SiteCakeMarks, SiteRetransmits, SiteRtt, SiteThroughput};

extern "C" {
    pub(crate) fn spawn_lts2(
        has_remote_host: bool,
        remote_host: *const c_char,
        license_key: *const c_char,
        node_id: *const c_char,
        node_name: *const c_char,
    ) -> c_int;

    pub(crate) fn update_license_status(
        has_remote_host: bool,
        remote_host: *const c_char,
        license_key: *const c_char,
        node_id: *const c_char,
        node_name: *const c_char,
    ) -> c_int;

    pub(crate) fn request_free_trial(
        name: *const c_char,
        email: *const c_char,
        business_name: *const c_char,
        address1: *const c_char,
        address2: *const c_char,
        city: *const c_char,
        state: *const c_char,
        zip: *const c_char,
        country: *const c_char,
        phone: *const c_char,
        website: *const c_char,
    ) -> *mut c_char;

    pub(crate) fn submit_network_tree(
        timestamp: u64,
        tree: *const u8,
        tree_length: usize,
    ) -> c_int;

    pub(crate) fn submit_shaped_devices(
        timestamp: u64,
        devices: *const u8,
        devices_length: usize,
    ) -> c_int;

    pub(crate) fn submit_total_throughput(
        timestamp: u64,
        download_bytes: u64,
        upload_bytes: u64,
        shaped_download_bytes: u64,
        shaped_upload_bytes: u64,
        packets_down: u64,
        packets_up: u64,
        tcp_packets_down: u64,
        tcp_packets_up: u64,
        udp_packets_down: u64,
        udp_packets_up: u64,
        icmp_packets_down: u64,
        icmp_packets_up: u64,
        has_max_rtt: bool,
        max_rtt: f32,
        has_min_rtt: bool,
        min_rtt: f32,
        has_median_rtt: bool,
        median_rtt: f32,
        tcp_retransmits_down: i32,
        tcp_retransmits_up: i32,
        cake_marks_down: i32,
        cake_marks_up: i32,
        cake_drops_down: i32,
        cake_drops_up: i32,
    ) -> c_int;

    pub(crate) fn submit_circuit_throughput_batch(
        buffer: *const CircuitThroughput,
        length: usize,
    ) -> c_int;

    pub(crate) fn submit_circuit_retransmits_batch(
        buffer: *const CircuitRetransmits,
        length: usize,
    ) -> c_int;

    pub(crate) fn submit_circuit_rtt_batch(
        buffer: *const CircuitRtt,
        length: usize,
    ) -> c_int;

    pub(crate) fn submit_circuit_cake_drops_batch(
        buffer: *const CircuitCakeDrops,
        length: usize,
    ) -> c_int;

    pub(crate) fn submit_circuit_cake_marks_batch(
        buffer: *const CircuitCakeMarks,
        length: usize,
    ) -> c_int;

    pub(crate) fn submit_site_throughput_batch(
        buffer: *const SiteThroughput,
        length: usize,
    ) -> c_int;

    pub(crate) fn submit_site_cake_drops_batch(
        buffer: *const SiteCakeDrops,
        length: usize,
    ) -> c_int;

    pub(crate) fn submit_site_cake_marks_batch(
        buffer: *const SiteCakeMarks,
        length: usize,
    ) -> c_int;

    pub(crate) fn submit_site_retransmits_batch(
        buffer: *const SiteRetransmits,
        length: usize,
    ) -> c_int;

    pub(crate) fn submit_site_rtt_batch(
        buffer: *const SiteRtt,
        length: usize,
    ) -> c_int;

    pub(crate) fn submit_shaper_utilization(
        tick: u64,
        average_cpu: f32,
        peak_cpu: f32,
        memory_percent: f32,
    ) -> c_int;

    pub(crate) fn get_lts_license_status() -> i32;
    pub(crate) fn get_lts_license_trial_remaining() -> i32;
    pub(crate) fn ingest_batch_complete();

    pub(crate) fn one_way_flow(
        start_time: u64,
        end_time: u64,
        local_ip: *const u8,
        remote_ip: *const u8,
        protocol: u8,
        dst_port: u16,
        src_port: u16,
        bytes: u64,
        circuit_hash: i64,
    );

    pub(crate) fn two_way_flow(
        start_time: u64,
        end_time: u64,
        local_ip: *const u8,
        remote_ip: *const u8,
        protocol: u8,
        dst_port: u16,
        src_port: u16,
        bytes_down: u64,
        bytes_up: u64,
        retransmit_times_down: *const u64,
        retransmit_times_length: u64,
        retransmit_times_up: *const u64,
        retransmit_times_up_length: u64,
        rtt1: f32,
        rtt2: f32,
        circuit_hash: i64,
    );

    pub(crate) fn allow_subnet(ip: *const c_char);
    pub(crate) fn ignore_subnet(ip: *const c_char);
    pub(crate) fn submit_blackboard(bytes: *const u8, length: usize);

    pub(crate) fn flow_count(timestamp: u64, count: u64);
}