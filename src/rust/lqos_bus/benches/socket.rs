//! Benchmarks for JSON serialization and gathering data from TC.
//! Please select an interface in `test_interface.txt` (no enter character
//! at the end). This benchmark will destructively clear and then create
//! TC queues - so don't select an interface that you need!

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use lqos_bus::*;

pub fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("encode_request", |b| {
        let session_to_encode = BusSession {
            auth_cookie: 1234,
            requests: vec![BusRequest::Ping],
        };
        b.iter(|| {
            let msg = encode_request(&session_to_encode).unwrap();
            black_box(msg);
        });
    });

    c.bench_function("decode_request", |b| {
        let session_to_encode = BusSession {
            auth_cookie: 1234,
            requests: vec![BusRequest::Ping],
        };
        let msg = encode_request(&session_to_encode).unwrap();
        b.iter(|| {
            let result = decode_request(&msg).unwrap();
            black_box(result);
        });
    });

    c.bench_function("encode_reply", |b| {
        let reply_to_encode = BusReply {
            auth_cookie: cookie_value(),
            responses: vec![ BusResponse::Ack ]
        };
        b.iter(|| {
            let result = encode_response(&reply_to_encode).unwrap();
            black_box(result);
        });
    });

    c.bench_function("decode_reply", |b| {
        let reply_to_encode = BusReply {
            auth_cookie: cookie_value(),
            responses: vec![ BusResponse::Ack ]
        };
        let msg = encode_response(&reply_to_encode).unwrap();
        b.iter(|| {
            let result = decode_response(&msg).unwrap();
            black_box(result);
        });
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
