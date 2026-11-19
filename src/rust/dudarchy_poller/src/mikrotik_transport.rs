//! The RouterOS API transport boundary: the TCP connection, both login
//! forms — the modern plain exchange and the bounded legacy
//! challenge-response continuation — and the framed sentence exchange —
//! connect, write, read, the multi-sentence reply loop, and the deadline
//! that bounds every step.
//!
//! This module is the whole socket surface of the RouterOS client. Every
//! I/O step is an await point bounded by its own deadline, and the caller's
//! cancellation check runs before each login write, between the login reply
//! phases, and between reply sentences, surfacing as
//! [`MikrotikError::Cancelled`]. The connection is owned by one exchange:
//! any error — cancellation included — drops it, so a cancelled framed
//! exchange is never reused and the next request reconnects from scratch.
//! The protocol half (framing, word and sentence parsing) lives in
//! [`crate::mikrotik_proto`], untouched.

use std::net::SocketAddr;
use std::time::Duration;

use md5::{Digest, Md5};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::mikrotik::{MikrotikError, Password};
use crate::mikrotik_proto::{MAX_SENTENCE_WORDS, encode_word, message_of, word_length};

/// Standard RouterOS API port, used when a target address carries no port.
const API_PORT: u16 = 8728;

/// Most neighbour rows one `/ip/neighbor/print` reply may carry. The
/// neighbour table is one `!re` sentence per row and can exceed any
/// fixed sentence count, so the row read is bounded here instead: a peer
/// streaming rows past any real table stops the read.
pub const MAX_NEIGHBOUR_ROWS: usize = 4096;

/// The cancellation check handed down from the collectors: consulted
/// before each login write, between the login reply phases, and before
/// every reply sentence.
type Cancel<'a> = &'a (dyn Fn() -> bool + Send + Sync);

/// Connects to `address` with `timeout` bounding the connect itself. The
/// stream's socket options need no timeouts: every awaited read and write
/// below is wrapped in its own deadline instead.
pub(crate) async fn connect(address: &str, timeout: Duration) -> Result<TcpStream, MikrotikError> {
    let socket = resolve_socket(address).await?;
    let connect = async {
        TcpStream::connect(socket)
            .await
            .map_err(|source| MikrotikError::Io {
                address: address.to_owned(),
                source,
            })
    };
    deadline(connect, address, timeout).await
}

/// Parses `address` as `host:port`, defaulting the port to the RouterOS
/// API port, and resolves hostnames via the runtime's resolver (numeric
/// addresses short-circuit before any lookup, as before).
async fn resolve_socket(address: &str) -> Result<SocketAddr, MikrotikError> {
    let into_io = |source: std::io::Error| MikrotikError::Io {
        address: address.to_owned(),
        source,
    };
    if let Ok(parsed) = address.parse() {
        return Ok(parsed);
    }
    let with_port = format!("{address}:{API_PORT}");
    let mut addrs = tokio::net::lookup_host(with_port.as_str())
        .await
        .map_err(into_io)?;
    addrs.next().ok_or_else(|| {
        into_io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no addresses found",
        ))
    })
}

/// Performs the RouterOS API login. The first sentence is the modern plain
/// form: `/login` with `=name=`/`=password=`, answered by `!done`.
///
/// A `!done` carrying exactly one `=ret=` challenge is the pre-6.43
/// legacy exchange, not a finished login: the challenge is decoded, the
/// password is hashed with it, and one `/login` with `=name=` and
/// `=response=` is sent. Only a final `!done` without a challenge
/// completes authentication; a second challenge is a protocol fault, never
/// success and never a loop.
///
/// A `!trap`/`!fatal` reply at either phase is
/// [`MikrotikError::LoginFailed`] carrying `=message=` when present. A
/// malformed, duplicated, or repeated challenge is a fixed
/// [`MikrotikError::Protocol`] diagnostic that never echoes the value.
/// Cancellation is consulted before each login write and between the reply
/// phases.
pub(crate) async fn login(
    stream: &mut TcpStream,
    address: &str,
    username: &str,
    password: &Password,
    timeout: Duration,
    cancel: Cancel<'_>,
) -> Result<(), MikrotikError> {
    if cancel() {
        return Err(MikrotikError::Cancelled {
            address: address.to_owned(),
        });
    }
    let sentence = [
        "/login".to_owned(),
        format!("=name={username}"),
        format!("=password={}", password.0),
    ];
    write_sentence(stream, &sentence, address, timeout).await?;
    let reply = read_sentence(stream, address, timeout).await?;
    let challenge = match login_reply(address, &reply)? {
        LoginReply::Done => return Ok(()),
        LoginReply::Rejected => return Err(login_failed(address, &reply)),
        LoginReply::Challenge(challenge) => challenge,
    };
    if cancel() {
        return Err(MikrotikError::Cancelled {
            address: address.to_owned(),
        });
    }
    let sentence = [
        "/login".to_owned(),
        format!("=name={username}"),
        legacy_response(password, &challenge),
    ];
    write_sentence(stream, &sentence, address, timeout).await?;
    if cancel() {
        return Err(MikrotikError::Cancelled {
            address: address.to_owned(),
        });
    }
    let reply = read_sentence(stream, address, timeout).await?;
    match login_reply(address, &reply)? {
        LoginReply::Done => Ok(()),
        LoginReply::Rejected => Err(login_failed(address, &reply)),
        LoginReply::Challenge(_) => Err(protocol_error(address, "login challenge repeated")),
    }
}

/// One login reply sentence, reduced to what the login state machine acts
/// on.
enum LoginReply {
    /// `!done` without a challenge: authentication finished.
    Done,
    /// `!trap`/`!fatal`: the router or account refused the login.
    Rejected,
    /// `!done` carrying the legacy challenge, decoded.
    Challenge([u8; 16]),
}

/// Classifies one login reply sentence. A `!done` with a single legacy
/// `=ret=` challenge is negotiation; malformed or duplicated challenges are
/// fixed protocol errors, and any other sentence is unexpected.
fn login_reply(address: &str, reply: &[String]) -> Result<LoginReply, MikrotikError> {
    match reply.first().map(String::as_str) {
        Some("!done") => match challenge_of(address, reply)? {
            None => Ok(LoginReply::Done),
            Some(challenge) => Ok(LoginReply::Challenge(challenge)),
        },
        Some("!trap" | "!fatal") => Ok(LoginReply::Rejected),
        _ => Err(protocol_error(address, "unexpected reply to login")),
    }
}

/// The legacy challenge carried by the sentence's `=ret=` attribute, when
/// present. Exactly one attribute of exactly 32 hex digits is a challenge;
/// a duplicate or malformed attribute is a fixed protocol error that never
/// echoes the offending value.
fn challenge_of(address: &str, reply: &[String]) -> Result<Option<[u8; 16]>, MikrotikError> {
    let mut values = reply.iter().filter_map(|word| word.strip_prefix("=ret="));
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(protocol_error(
            address,
            "login reply carries duplicate challenges",
        ));
    }
    decode_challenge(value)
        .map(Some)
        .ok_or_else(|| protocol_error(address, "login challenge is not 32 hex digits"))
}

/// Decodes a legacy challenge of exactly 32 hex digits (16 bytes); both
/// cases decode identically.
fn decode_challenge(value: &str) -> Option<[u8; 16]> {
    let digits = value.as_bytes();
    if digits.len() != 32 {
        return None;
    }
    let mut decoded = [0u8; 16];
    for (byte, pair) in decoded.iter_mut().zip(digits.chunks_exact(2)) {
        *byte = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Some(decoded)
}

/// One hex digit's value, for both cases.
fn hex_nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

/// The legacy login response sentence word: MD5 over one `0x00` byte, the
/// password's UTF-8 bytes unchanged, and the decoded challenge bytes. The
/// `00` starting the wire value is a textual prefix on the digest, not part
/// of the hashed input.
fn legacy_response(password: &Password, challenge: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hash = Md5::new();
    hash.update([0u8]);
    hash.update(password.0.as_bytes());
    hash.update(challenge);
    let digest = hash.finalize();
    let mut response = String::with_capacity("=response=00".len() + digest.len() * 2);
    response.push_str("=response=00");
    for byte in digest {
        response.push(HEX[usize::from(byte >> 4)] as char);
        response.push(HEX[usize::from(byte & 0x0F)] as char);
    }
    response
}

/// A fixed protocol diagnostic; no reply value is ever echoed.
fn protocol_error(address: &str, reason: &'static str) -> MikrotikError {
    MikrotikError::Protocol {
        address: address.to_owned(),
        reason: reason.to_owned(),
    }
}

/// A rejected login carrying the router's `=message=` when present.
fn login_failed(address: &str, reply: &[String]) -> MikrotikError {
    MikrotikError::LoginFailed {
        address: address.to_owned(),
        message: message_of(reply).to_owned(),
    }
}

/// Sends `command` and reads reply sentences until `!done`, returning one
/// sentence per `!re`, bounded by `row_cap`. A `!trap`/`!fatal` reply is
/// [`MikrotikError::CommandFailed`] carrying `=message=` when present.
pub(crate) async fn request_sentences(
    stream: &mut TcpStream,
    address: &str,
    command: &str,
    row_cap: usize,
    timeout: Duration,
    cancel: Cancel<'_>,
) -> Result<Vec<Vec<String>>, MikrotikError> {
    write_sentence(stream, &[command.to_owned()], address, timeout).await?;
    let mut rows = Vec::new();
    loop {
        if cancel() {
            return Err(MikrotikError::Cancelled {
                address: address.to_owned(),
            });
        }
        let reply = read_sentence(stream, address, timeout).await?;
        match reply.first().map(String::as_str) {
            Some("!re") => {
                rows.push(reply);
                if rows.len() > row_cap {
                    return Err(MikrotikError::Protocol {
                        address: address.to_owned(),
                        reason: "too many reply rows".to_owned(),
                    });
                }
            }
            Some("!done") => return Ok(rows),
            Some("!trap" | "!fatal") => {
                return Err(MikrotikError::CommandFailed {
                    address: address.to_owned(),
                    message: message_of(&reply).to_owned(),
                });
            }
            _ => {
                return Err(MikrotikError::Protocol {
                    address: address.to_owned(),
                    reason: "unexpected reply sentence".to_owned(),
                });
            }
        }
    }
}

/// Cap on rows read for one device-details print query, so a peer
/// streaming endless rows cannot grow a read unboundedly.
pub(crate) const MAX_PRINT_ROWS: usize = 4096;

/// Writes one sentence: each word length-prefixed, terminated by a
/// zero-length word. `timeout` bounds the write.
async fn write_sentence(
    stream: &mut TcpStream,
    words: &[String],
    address: &str,
    timeout: Duration,
) -> Result<(), MikrotikError> {
    let mut out = Vec::new();
    for word in words {
        encode_word(&mut out, word).map_err(|reason| MikrotikError::Protocol {
            address: address.to_owned(),
            reason: reason.to_owned(),
        })?;
    }
    out.push(0);
    let write = async {
        stream
            .write_all(&out)
            .await
            .map_err(|source| MikrotikError::Io {
                address: address.to_owned(),
                source,
            })
    };
    deadline(write, address, timeout).await
}

/// Reads one length-prefixed word. The length prefix is decoded by
/// [`word_length`]; longer forms are rejected there. `timeout` bounds the
/// read.
async fn read_word(
    stream: &mut TcpStream,
    address: &str,
    timeout: Duration,
) -> Result<Vec<u8>, MikrotikError> {
    let first = read_byte(stream, address, timeout).await?;
    let second = if (0x80..0xC0).contains(&first) {
        Some(read_byte(stream, address, timeout).await?)
    } else {
        None
    };
    let length = match word_length(first, second) {
        Ok(length) => length,
        Err(reason) => {
            return Err(MikrotikError::Protocol {
                address: address.to_owned(),
                reason: reason.to_owned(),
            });
        }
    };
    let mut word = vec![0u8; length];
    let fill = async {
        stream
            .read_exact(&mut word)
            .await
            .map_err(|source| MikrotikError::Io {
                address: address.to_owned(),
                source,
            })
    };
    deadline(fill, address, timeout).await?;
    Ok(word)
}

/// Reads exactly one byte, bounded by `timeout`.
async fn read_byte(
    stream: &mut TcpStream,
    address: &str,
    timeout: Duration,
) -> Result<u8, MikrotikError> {
    let read = async {
        let mut byte = [0u8; 1];
        stream
            .read_exact(&mut byte)
            .await
            .map(|_| byte[0])
            .map_err(|source| MikrotikError::Io {
                address: address.to_owned(),
                source,
            })
    };
    deadline(read, address, timeout).await
}

/// Bounds one I/O step by `timeout`: an expiry is the same
/// [`MikrotikError::Io`] a socket timeout produced before.
async fn deadline<F, T>(future: F, address: &str, timeout: Duration) -> Result<T, MikrotikError>
where
    F: std::future::Future<Output = Result<T, MikrotikError>>,
{
    match tokio::time::timeout(timeout, future).await {
        Ok(result) => result,
        Err(_elapsed) => Err(MikrotikError::Io {
            address: address.to_owned(),
            source: std::io::Error::new(std::io::ErrorKind::TimedOut, "operation timed out"),
        }),
    }
}

/// Reads one sentence: words until a zero-length word, each decoded as
/// UTF-8 and bounded by [`MAX_SENTENCE_WORDS`]. `timeout` bounds every
/// word read.
async fn read_sentence(
    stream: &mut TcpStream,
    address: &str,
    timeout: Duration,
) -> Result<Vec<String>, MikrotikError> {
    let into_protocol = |reason: &'static str| MikrotikError::Protocol {
        address: address.to_owned(),
        reason: reason.to_owned(),
    };
    let mut words = Vec::new();
    loop {
        let word = read_word(stream, address, timeout).await?;
        if word.is_empty() {
            return Ok(words);
        }
        if words.len() >= MAX_SENTENCE_WORDS {
            return Err(into_protocol("sentence has too many words"));
        }
        let word = String::from_utf8(word).map_err(|_| into_protocol("word is not valid UTF-8"))?;
        words.push(word);
    }
}
