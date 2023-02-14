//! Becnhmarks for the bus system, mostly focused on serialization
//! but also including sockets.
//!
//! You MUST have lqosd running when you perform these tests.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use lqos_bus::*;

pub fn criterion_benchmark(c: &mut Criterion) {
  c.bench_function("encode_request", |b| {
    let session_to_encode =
      BusSession { persist: false, requests: vec![BusRequest::Ping] };
    b.iter(|| {
      let msg = encode_request(&session_to_encode).unwrap();
      black_box(msg);
    });
  });

  c.bench_function("decode_request", |b| {
    let session_to_encode =
      BusSession { persist: false, requests: vec![BusRequest::Ping] };
    let msg = encode_request(&session_to_encode).unwrap();
    b.iter(|| {
      let result = decode_request(&msg).unwrap();
      black_box(result);
    });
  });

  c.bench_function("encode_reply", |b| {
    let reply_to_encode = BusReply { responses: vec![BusResponse::Ack] };
    b.iter(|| {
      let result = encode_response(&reply_to_encode).unwrap();
      black_box(result);
    });
  });

  c.bench_function("decode_reply", |b| {
    let reply_to_encode = BusReply { responses: vec![BusResponse::Ack] };
    let msg = encode_response(&reply_to_encode).unwrap();
    b.iter(|| {
      let result = decode_response(&msg).unwrap();
      black_box(result);
    });
  });

  // Enable the Tokio runtime to test round-trip
  let tokio_rt =
    tokio::runtime::Builder::new_current_thread().enable_io().build().unwrap();

  c.bench_function("bus_ping_round_trip", |b| {
    b.iter(|| {
      let result =
        tokio_rt.block_on(bus_request(vec![BusRequest::Ping])).unwrap();
      black_box(result);
    });
  });

  c.bench_function("bus_ping_with_persistence", |b| {
    let mut client = tokio_rt.block_on(BusClient::new()).unwrap();
    b.iter(|| {
      let result =
        tokio_rt.block_on(client.request(vec![BusRequest::Ping])).unwrap();
      black_box(result);
    });
  });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
