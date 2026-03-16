//! Helpers for configuring the Rustls crypto provider used by HTTPS clients.

use std::sync::OnceLock;

use thiserror::Error;

/// Errors returned while initializing the process-wide Rustls crypto provider.
#[derive(Debug, Error)]
pub enum RustlsProviderError {
    /// Installing the selected Rustls provider failed before one was available.
    #[error("failed to install the Rustls crypto provider")]
    InstallFailed,
}

/// Ensure a process-wide Rustls crypto provider is installed.
///
/// Side effects:
/// installs the default AWS-LC Rustls provider for the current process if one
/// has not already been installed.
pub fn ensure_rustls_crypto_provider() -> Result<(), RustlsProviderError> {
    static INIT: OnceLock<()> = OnceLock::new();

    if INIT.get().is_some() || rustls::crypto::CryptoProvider::get_default().is_some() {
        let _ = INIT.get_or_init(|| ());
        return Ok(());
    }

    if rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
        && rustls::crypto::CryptoProvider::get_default().is_none()
    {
        return Err(RustlsProviderError::InstallFailed);
    }
    let _ = INIT.get_or_init(|| ());
    Ok(())
}
