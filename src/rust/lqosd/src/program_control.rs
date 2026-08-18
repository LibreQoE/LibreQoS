use lqos_bus::BusResponse;
use tracing::info;

pub fn reload_libre_qos() -> BusResponse {
    let result = lqos_config::load_libreqos();
    match result {
        Ok(message) => BusResponse::ReloadLibreQoS(message),
        Err(..) => BusResponse::Fail("Unable to reload LibreQoS".to_string()),
    }
}

/// Requests the same graceful shutdown path used by systemd and operator SIGTERM.
///
/// Side effects: this function sends `SIGTERM` to the current process. The
/// signal handler owns cleanup for XDP/TC detach, bus socket cleanup, and file
/// lock release before process exit.
pub fn request_graceful_shutdown(reason: &str) -> anyhow::Result<()> {
    request_graceful_shutdown_with(reason, send_sigterm_to_self)
}

fn request_graceful_shutdown_with(
    reason: &str,
    send_sigterm: impl FnOnce() -> std::io::Result<()>,
) -> anyhow::Result<()> {
    info!("Requesting graceful lqosd shutdown: {reason}");
    send_sigterm()?;
    Ok(())
}

fn send_sigterm_to_self() -> std::io::Result<()> {
    let result = unsafe { nix::libc::kill(nix::libc::getpid(), nix::libc::SIGTERM) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[test]
    fn graceful_shutdown_request_sends_sigterm() {
        let called = Arc::new(AtomicBool::new(false));
        let called_by_sender = called.clone();

        request_graceful_shutdown_with("test", || {
            called_by_sender.store(true, Ordering::SeqCst);
            Ok(())
        })
        .expect("shutdown request should succeed");

        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    fn graceful_shutdown_request_returns_signal_errors() {
        let err = request_graceful_shutdown_with("test", || {
            Err(std::io::Error::from_raw_os_error(nix::libc::EPERM))
        })
        .expect_err("signal errors should be returned");

        let io_error = err
            .downcast_ref::<std::io::Error>()
            .expect("shutdown request should return the signal IO error");
        assert_eq!(io_error.raw_os_error(), Some(nix::libc::EPERM));
    }
}
