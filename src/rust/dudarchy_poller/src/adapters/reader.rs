//! The engine's only I/O seam: typed SNMP gets and walks, and named API
//! commands. A reader is a scripted transcript in tests and the live
//! transport in the product; the engine never opens a socket.

use std::future::Future;
use std::pin::Pin;

use crate::snmp_tables::WalkValue;

use crate::commands::CommandName;
/// One named-command row: the reply's key/value pairs.
pub use crate::commands::CommandRow;

/// One walked or fetched varbind.
#[derive(Clone, Debug, PartialEq)]
pub struct Varbind {
    /// The full dotted OID as arcs.
    pub oid: Vec<u32>,
    /// The decoded value.
    pub value: WalkValue,
}

/// A read's typed outcome. `Empty` is only produced for table and command
/// reads: a scalar read that answers nothing is `Absent`.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome<T> {
    /// The agent answered.
    Present(T),
    /// A table or command answered with zero rows: a successful answer.
    Empty,
    /// The object or table does not exist.
    Absent,
    /// No answer within the deadline.
    Timeout,
    /// The agent answered an error, or the reply did not decode.
    Error {
        /// What went wrong, in the reader's own words.
        reason: String,
    },
    /// The walk was cancelled before the read completed.
    Cancelled,
}

impl<T> Outcome<T> {
    /// The outcome without its payload.
    pub fn kind(&self) -> OutcomeKind {
        match self {
            Self::Present(_) => OutcomeKind::Present,
            Self::Empty => OutcomeKind::Empty,
            Self::Absent => OutcomeKind::Absent,
            Self::Timeout => OutcomeKind::Timeout,
            Self::Error { .. } => OutcomeKind::Error,
            Self::Cancelled => OutcomeKind::Cancelled,
        }
    }

    /// The failure reason, when the outcome is an error.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Error { reason } => Some(reason),
            _ => None,
        }
    }
}

/// The typed outcome, without its payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeKind {
    /// The agent answered.
    Present,
    /// A table or command answered with zero rows.
    Empty,
    /// The object or table does not exist.
    Absent,
    /// No answer within the deadline.
    Timeout,
    /// The agent answered an error, or the reply did not decode.
    Error,
    /// The walk was cancelled before the read completed.
    Cancelled,
}

/// The engine's I/O seam. Implementations answer from a live transport or a
/// recorded transcript; the futures are boxed so the trait stays
/// dyn-compatible, matching the walk's own source seam.
pub trait Reader {
    /// Reads one scalar OID.
    fn get<'a>(
        &'a mut self,
        oid: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>>;

    /// Walks one subtree.
    fn walk<'a>(
        &'a mut self,
        root: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>>;

    /// Runs one core-registered read-only named command.
    fn command<'a>(
        &'a mut self,
        name: CommandName,
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<CommandRow>>> + Send + 'a>>;
}
