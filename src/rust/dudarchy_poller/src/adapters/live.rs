//! The live readers: the engine's [`Reader`] over the real SNMP client and
//! over the real RouterOS API client.
//!
//! One reader serves one device. Each carries the target address, the
//! credential the core resolved from the vault, the deadline for one read,
//! and the job's cancellation flag. Neither reads the vault or sees a
//! document secret.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::mikrotik::{MikrotikError, command};
use crate::snmp::{GetError, SnmpError, Walked, get_oid, walk_values};
use crate::walk::{ApiCredentials, SnmpAccess};

use super::CommandName;
use super::reader::{CommandRow, Outcome, Reader, Varbind};

/// How often an in-flight read notices the cancellation flag. The flag is a
/// plain atomic, and 20 ms is orders of magnitude below any SNMP deadline.
const CANCEL_POLL: Duration = Duration::from_millis(20);

/// The engine's reader over the live SNMP transport.
///
/// The typed mapping is the engine's: an answered GET, or a walk with rows,
/// is `Present`; a v2c `noSuchObject`/`noSuchInstance` on the requested
/// object is `Absent`; a walk that ended with zero rows is `Empty`; a missed
/// deadline is `Timeout`; a transport, decode, or agent error is `Error`
/// with its reason; and a risen cancellation flag is `Cancelled`, including
/// while a request is in flight. SNMP v1 has no exception values, so a v1
/// agent's `noSuchName` ends a walk as an empty table — the dialect the core
/// walk has always reported.
///
/// Named commands are RouterOS API operations, not SNMP: the reader reports
/// them as an error rather than inventing a query.
pub struct SnmpReader<'a> {
    /// The target's address, as the vault and walk use it.
    address: String,
    /// The community and SNMP version the core resolved for this device.
    access: &'a SnmpAccess,
    /// The deadline for one read: one request, or one walk step's exchange.
    timeout: Duration,
    /// The job's cancellation flag, polled between reads and while one runs.
    cancel: &'a (dyn Fn() -> bool + Send + Sync),
}

impl<'a> SnmpReader<'a> {
    /// A reader for one device. `access` is already resolved from the vault;
    /// nothing here reads a credential, and the borrow keeps the resolved
    /// access with the credential candidate that produced it.
    pub fn new(
        address: impl Into<String>,
        access: &'a SnmpAccess,
        timeout: Duration,
        cancel: &'a (dyn Fn() -> bool + Send + Sync),
    ) -> SnmpReader<'a> {
        SnmpReader {
            address: address.into(),
            access,
            timeout,
            cancel,
        }
    }

    /// Whether the job was cancelled before a read starts.
    fn cancelled(&self) -> bool {
        (self.cancel)()
    }
}

impl Reader for SnmpReader<'_> {
    fn get<'a>(
        &'a mut self,
        oid: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
        Box::pin(async move {
            if self.cancelled() {
                return Outcome::Cancelled;
            }
            let answer = tokio::select! {
                answer = get_oid(
                    &self.address,
                    &self.access.community,
                    self.access.version,
                    oid,
                    self.timeout,
                ) => answer,
                _ = wait_cancelled(self.cancel) => return Outcome::Cancelled,
            };
            match answer {
                Ok(Some((oid, value))) => Outcome::Present(vec![Varbind { oid, value }]),
                Ok(None) => Outcome::Absent,
                Err(GetError::Snmp(error)) => failure(error),
                Err(other) => Outcome::Error {
                    reason: other.to_string(),
                },
            }
        })
    }

    fn walk<'a>(
        &'a mut self,
        root: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
        Box::pin(async move {
            if self.cancelled() {
                return Outcome::Cancelled;
            }
            let answer = tokio::select! {
                answer = walk_values(
                    &self.address,
                    &self.access.community,
                    root,
                    self.access.version,
                    self.timeout,
                    self.cancel,
                ) => answer,
                _ = wait_cancelled(self.cancel) => return Outcome::Cancelled,
            };
            match answer {
                Ok(Walked::Rows(rows)) if rows.is_empty() => Outcome::Empty,
                Ok(Walked::Rows(rows)) => Outcome::Present(
                    rows.into_iter()
                        .map(|(oid, value)| Varbind { oid, value })
                        .collect(),
                ),
                Ok(Walked::NoSuchObject) => Outcome::Absent,
                Ok(Walked::Cancelled(_)) => Outcome::Cancelled,
                Err(error) => failure(error),
            }
        })
    }

    fn command<'a>(
        &'a mut self,
        _name: CommandName,
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<CommandRow>>> + Send + 'a>> {
        Box::pin(async move {
            Outcome::Error {
                reason: "named commands are RouterOS API operations, not SNMP".to_owned(),
            }
        })
    }
}

/// The engine's reader over the live RouterOS API transport.
///
/// One reader serves one device and one authenticated credential. Each named
/// command opens its own connection, logs in, and reads the reply rows; the
/// configured pacing delay precedes every command, and the job's
/// cancellation flag is consulted before each one and by the exchange
/// itself. SNMP reads are not API operations, so `get` and `walk` report an
/// error instead of inventing a query.
pub struct ApiReader<'a> {
    /// The target's address, as the vault and walk use it.
    address: String,
    /// The username and password the core resolved for this device.
    credentials: &'a ApiCredentials,
    /// The deadline for one exchange: connect, login, and the command.
    timeout: Duration,
    /// The discovery pacing delay before each command.
    pace: Duration,
    /// The job's cancellation flag.
    cancel: &'a (dyn Fn() -> bool + Send + Sync),
}

impl<'a> ApiReader<'a> {
    /// A reader for one device. `credentials` is already resolved from the
    /// vault; nothing here reads a credential.
    pub fn new(
        address: impl Into<String>,
        credentials: &'a ApiCredentials,
        timeout: Duration,
        pace: Duration,
        cancel: &'a (dyn Fn() -> bool + Send + Sync),
    ) -> ApiReader<'a> {
        ApiReader {
            address: address.into(),
            credentials,
            timeout,
            pace,
            cancel,
        }
    }
}

impl Reader for ApiReader<'_> {
    fn get<'a>(
        &'a mut self,
        _oid: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
        Box::pin(async move {
            Outcome::Error {
                reason: "SNMP reads are not RouterOS API operations".to_owned(),
            }
        })
    }

    fn walk<'a>(
        &'a mut self,
        _root: &'a [u32],
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<Varbind>>> + Send + 'a>> {
        Box::pin(async move {
            Outcome::Error {
                reason: "SNMP reads are not RouterOS API operations".to_owned(),
            }
        })
    }

    fn command<'a>(
        &'a mut self,
        name: CommandName,
    ) -> Pin<Box<dyn Future<Output = Outcome<Vec<CommandRow>>> + Send + 'a>> {
        Box::pin(async move {
            if (self.cancel)() {
                return Outcome::Cancelled;
            }
            tokio::time::sleep(self.pace).await;
            match command(
                &self.address,
                &self.credentials.username,
                &self.credentials.password,
                name,
                self.timeout,
                self.cancel,
            )
            .await
            {
                Ok(rows) if rows.is_empty() => Outcome::Empty,
                Ok(rows) => Outcome::Present(rows),
                Err(MikrotikError::Cancelled { .. }) => Outcome::Cancelled,
                Err(error) => Outcome::Error {
                    reason: error.to_string(),
                },
            }
        })
    }
}

/// One transport failure as the engine's typed outcome.
fn failure<T>(error: SnmpError) -> Outcome<T> {
    match error {
        SnmpError::Timeout { .. } => Outcome::Timeout,
        SnmpError::Cancelled { .. } => Outcome::Cancelled,
        other => Outcome::Error {
            reason: other.to_string(),
        },
    }
}

/// Completes when the cancellation flag rises, polled at [`CANCEL_POLL`]:
/// the flag is a plain atomic, and a 20 ms bound on noticing it is orders of
/// magnitude below any SNMP deadline.
async fn wait_cancelled(cancel: &(dyn Fn() -> bool + Send + Sync)) {
    let mut tick = tokio::time::interval(CANCEL_POLL);
    tick.tick().await; // the first tick fires at once; skip it
    loop {
        tick.tick().await;
        if cancel() {
            return;
        }
    }
}
