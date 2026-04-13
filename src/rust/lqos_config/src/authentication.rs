//! The `authentication` module provides authorization for use of the
//! local web UI on LibreQoS boxes. It maps to `/<install dir>/lqusers.toml`

use allocative::Allocative;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt::Display,
    fs::{OpenOptions, read_to_string, remove_file, rename},
    io::Write,
    path::{Path, PathBuf},
};
use thiserror::Error;
use tracing::{error, warn};
use uuid::Uuid;

const AUTH_FILE_VERSION: u32 = 2;
const LEGACY_AUTH_FILE_VERSION: u32 = 1;
const INITIAL_AUTH_EPOCH: u64 = 1;
const LEGACY_AUTH_FILE_NAME: &str = "webusers.toml";
const CURRENT_AUTH_FILE_NAME: &str = "lqusers.toml";
const LEGACY_PASSWORD_PEPPER: &str = "_LibreQosLikesPasswordsForDinner";

fn default_auth_file_version() -> u32 {
    LEGACY_AUTH_FILE_VERSION
}

fn default_auth_epoch() -> u64 {
    INITIAL_AUTH_EPOCH
}

/// Access rights of a user
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Allocative)]
pub enum UserRole {
    /// The user may view data but not change it.
    ReadOnly,
    /// The user may make any changes they request.
    Admin,
}

impl From<&str> for UserRole {
    fn from(s: &str) -> Self {
        let s = s.to_lowercase();
        if s == "admin" {
            UserRole::Admin
        } else {
            UserRole::ReadOnly
        }
    }
}

impl From<String> for UserRole {
    fn from(s: String) -> Self {
        let s = s.to_lowercase();
        if s == "admin" {
            UserRole::Admin
        } else {
            UserRole::ReadOnly
        }
    }
}

impl Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserRole::Admin => write!(f, "admin"),
            UserRole::ReadOnly => write!(f, "read-only"),
        }
    }
}

/// A user of the web UI.
#[derive(Clone, Debug, Deserialize, Serialize, Allocative)]
pub struct WebUser {
    /// The user's username.
    pub username: String,
    /// The user's password hash. This may be a legacy SHA-256 hash or an
    /// Argon2id PHC string until the user next logs in.
    pub password_hash: String,
    /// The user's role.
    pub role: UserRole,
}

/// Result of authenticating a single user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedUser {
    /// The authenticated username.
    pub username: String,
    /// The user's role.
    pub role: UserRole,
    /// Current auth epoch after any migration-side rewrite.
    pub auth_epoch: u64,
    /// True when a legacy password hash was upgraded to Argon2id.
    pub password_upgraded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PasswordVerification {
    valid: bool,
    needs_rehash: bool,
}

/// Container holding the authorized web users.
#[derive(Clone, Debug, Deserialize, Serialize, Allocative)]
pub struct WebUsers {
    #[serde(default = "default_auth_file_version")]
    version: u32,
    #[serde(default = "default_auth_epoch")]
    auth_epoch: u64,
    #[serde(default)]
    allow_unauthenticated_to_view: bool,
    #[serde(default)]
    users: Vec<WebUser>,
}

impl Default for WebUsers {
    fn default() -> Self {
        Self {
            version: AUTH_FILE_VERSION,
            auth_epoch: INITIAL_AUTH_EPOCH,
            allow_unauthenticated_to_view: false,
            users: Vec::new(),
        }
    }
}

impl WebUsers {
    fn base_path() -> Result<PathBuf, AuthenticationError> {
        let base_path = crate::load_config()
            .map_err(|_| AuthenticationError::UnableToLoadEtcLqos)?
            .lqos_directory
            .clone();
        Ok(PathBuf::from(base_path))
    }

    fn primary_path() -> Result<PathBuf, AuthenticationError> {
        Ok(Self::base_path()?.join(CURRENT_AUTH_FILE_NAME))
    }

    fn legacy_path() -> Result<PathBuf, AuthenticationError> {
        Ok(Self::base_path()?.join(LEGACY_AUTH_FILE_NAME))
    }

    /// Returns the current `lqusers.toml` path.
    pub fn path() -> Result<PathBuf, AuthenticationError> {
        Self::primary_path()
    }

    /// Returns the existing auth file path, checking the legacy filename as a fallback.
    pub fn existing_path() -> Result<Option<PathBuf>, AuthenticationError> {
        let current = Self::primary_path()?;
        if current.exists() {
            return Ok(Some(current));
        }

        let legacy = Self::legacy_path()?;
        if legacy.exists() {
            return Ok(Some(legacy));
        }

        Ok(None)
    }

    /// Is the list of users empty?
    pub fn is_empty(&self) -> bool {
        self.users.is_empty()
    }

    fn normalize_for_save(&self) -> Self {
        let mut normalized = self.clone();
        normalized.version = AUTH_FILE_VERSION;
        if normalized.auth_epoch == 0 {
            normalized.auth_epoch = INITIAL_AUTH_EPOCH;
        }
        normalized
    }

    fn save_to_disk(&self) -> Result<(), AuthenticationError> {
        let path = Self::primary_path()?;
        let tmp_path = path.with_extension(format!("toml.tmp-{}", Uuid::new_v4()));
        let normalized = self.normalize_for_save();
        let new_contents = toml_edit::ser::to_string(&normalized)
            .map_err(AuthenticationError::SerializationError)?;

        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .map_err(|e| {
                error!(
                    "Unable to open temporary auth file {:?} for writing: {e}",
                    tmp_path
                );
                AuthenticationError::UnableToWrite
            })?;
        file.write_all(new_contents.as_bytes()).map_err(|e| {
            error!("Unable to write temporary auth file {:?}: {e}", tmp_path);
            AuthenticationError::UnableToWrite
        })?;
        drop(file);

        rename(&tmp_path, &path).map_err(|e| {
            error!(
                "Unable to rename temporary auth file {:?} to {:?}: {e}",
                tmp_path, path
            );
            let _ = remove_file(&tmp_path);
            AuthenticationError::UnableToWrite
        })?;

        let legacy_path = Self::legacy_path()?;
        if legacy_path != path
            && legacy_path.exists()
            && let Err(e) = remove_file(&legacy_path)
        {
            warn!("Unable to remove legacy auth file {:?}: {e}", legacy_path);
        }

        Ok(())
    }

    fn migrate_if_needed(&mut self, loaded_from: &Path) -> Result<(), AuthenticationError> {
        let current_path = Self::primary_path()?;
        let loaded_from_legacy_path = loaded_from != current_path;
        let mut needs_rewrite = false;

        if self.version != AUTH_FILE_VERSION {
            self.version = AUTH_FILE_VERSION;
            needs_rewrite = true;
        }

        if self.auth_epoch == 0 {
            self.auth_epoch = INITIAL_AUTH_EPOCH;
            needs_rewrite = true;
        }

        if loaded_from_legacy_path {
            needs_rewrite = true;
        }

        if needs_rewrite {
            self.bump_auth_epoch();
            self.save_to_disk()?;
        }

        Ok(())
    }

    /// Does the user's file exist? True if it does, false otherwise.
    pub fn does_users_file_exist() -> Result<bool, AuthenticationError> {
        Ok(Self::existing_path()?.is_some())
    }

    /// Try to load `lqusers.toml`, creating a new version 2 file if no auth file exists.
    pub fn load_or_create() -> Result<Self, AuthenticationError> {
        if let Some(path) = Self::existing_path()? {
            let raw = read_to_string(&path).map_err(|e| {
                error!("Unable to read auth file {:?}: {e}", path);
                AuthenticationError::UnableToRead
            })?;
            let mut users: Self = toml_edit::de::from_str(&raw).map_err(|e| {
                error!("Unable to deserialize auth file {:?}: {e}", path);
                AuthenticationError::UnableToParse
            })?;
            users.migrate_if_needed(&path)?;
            Ok(users)
        } else {
            let new_users = Self::default();
            new_users.save_to_disk()?;
            Ok(new_users)
        }
    }

    fn hash_password(password: &str) -> Result<String, AuthenticationError> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(AuthenticationError::PasswordHashError)
    }

    fn hash_password_legacy(password: &str) -> String {
        let salted = format!("!x{password}{LEGACY_PASSWORD_PEPPER}");
        let mut sha256 = Sha256::new();
        sha256.update(salted);
        crate::hex_encoding::encode_hex_upper(sha256.finalize())
    }

    fn verify_password(
        password: &str,
        password_hash: &str,
    ) -> Result<PasswordVerification, AuthenticationError> {
        if password_hash.starts_with("$argon2") {
            let parsed =
                PasswordHash::new(password_hash).map_err(AuthenticationError::PasswordHashError)?;
            Ok(PasswordVerification {
                valid: Argon2::default()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok(),
                needs_rehash: false,
            })
        } else {
            Ok(PasswordVerification {
                valid: Self::hash_password_legacy(password) == password_hash,
                needs_rehash: true,
            })
        }
    }

    fn bump_auth_epoch(&mut self) {
        self.auth_epoch = self.auth_epoch.saturating_add(1).max(INITIAL_AUTH_EPOCH);
    }

    /// Returns the current auth epoch used to revoke signed sessions.
    pub fn auth_epoch(&self) -> u64 {
        self.auth_epoch
    }

    /// If a user exists with this username, update their details to the
    /// provided values. If the user does not exist, create them with the
    /// provided values.
    pub fn add_or_update_user(
        &mut self,
        username: &str,
        password: &str,
        role: UserRole,
    ) -> Result<(), AuthenticationError> {
        let password_hash = Self::hash_password(password)?;
        if let Some(user) = self.users.iter_mut().find(|u| u.username == username) {
            user.password_hash = password_hash;
            user.role = role;
        } else {
            let new_user = WebUser {
                username: username.to_string(),
                password_hash,
                role,
            };
            self.users.push(new_user);
        }

        self.bump_auth_epoch();
        self.save_to_disk()?;
        Ok(())
    }

    /// Update an existing user, optionally changing their password.
    ///
    /// If `password` is `Some`, the password hash is updated; if it is `None`,
    /// the existing password hash is left unchanged. The user's role is always
    /// updated. This function does not create a new user; attempting to update
    /// a non-existent user returns [`AuthenticationError::UserNotFound`].
    pub fn update_user_with_optional_password(
        &mut self,
        username: &str,
        password: Option<&str>,
        role: UserRole,
    ) -> Result<(), AuthenticationError> {
        let Some(user) = self.users.iter_mut().find(|u| u.username == username) else {
            return Err(AuthenticationError::UserNotFound);
        };

        if let Some(password) = password {
            user.password_hash = Self::hash_password(password)?;
        }
        user.role = role;
        self.bump_auth_epoch();
        self.save_to_disk()?;
        Ok(())
    }

    /// Delete a user from `lqusers.toml`
    pub fn remove_user(&mut self, username: &str) -> Result<(), AuthenticationError> {
        let old_len = self.users.len();
        self.users.retain(|u| u.username != username);
        if old_len == self.users.len() {
            error!("User {username} not found, hence not deleted.");
            return Err(AuthenticationError::UserNotFound);
        }
        self.bump_auth_epoch();
        self.save_to_disk()?;
        Ok(())
    }

    /// Attempt a login with the specified username and password. If the login
    /// succeeds, returns the authenticated user details and transparently
    /// upgrades legacy password hashes to Argon2id.
    pub fn authenticate(
        &mut self,
        username: &str,
        password: &str,
    ) -> Result<AuthenticatedUser, AuthenticationError> {
        let Some(index) = self.users.iter().position(|u| u.username == username) else {
            return Err(AuthenticationError::InvalidLogin);
        };

        let verification = Self::verify_password(password, &self.users[index].password_hash)?;
        if !verification.valid {
            return Err(AuthenticationError::InvalidLogin);
        }

        let mut password_upgraded = false;
        if verification.needs_rehash {
            self.users[index].password_hash = Self::hash_password(password)?;
            self.bump_auth_epoch();
            self.save_to_disk()?;
            password_upgraded = true;
        }

        Ok(AuthenticatedUser {
            username: self.users[index].username.clone(),
            role: self.users[index].role,
            auth_epoch: self.auth_epoch,
            password_upgraded,
        })
    }

    /// Dump all users to the console.
    pub fn print_users(&self) -> Result<(), AuthenticationError> {
        self.users.iter().for_each(|u| {
            println!("{:<40} {:<10}", u.username, u.role.to_string());
        });
        Ok(())
    }

    /// Return a list of user objects
    pub fn get_users(&self) -> Vec<WebUser> {
        self.users.clone()
    }

    /// Sets the "allow unauthenticated users" field. If true,
    /// unauthenticated users gain read-only access. This is useful
    /// for demonstration purposes.
    pub fn allow_anonymous(&mut self, allow: bool) -> Result<(), AuthenticationError> {
        self.allow_unauthenticated_to_view = allow;
        self.bump_auth_epoch();
        self.save_to_disk()?;
        Ok(())
    }

    /// Do we allow unauthenticated users to read site data?
    pub fn do_we_allow_anonymous(&self) -> bool {
        self.allow_unauthenticated_to_view
    }
}

/// Errors that can occur while managing web-UI authentication.
#[derive(Error, Debug)]
pub enum AuthenticationError {
    /// Failed to load the main configuration (`/etc/lqos.conf`) to
    /// resolve the LibreQoS directory path.
    #[error("Unable to load /etc/lqos.conf")]
    UnableToLoadEtcLqos,
    /// Failed to serialize [`WebUsers`] to TOML before writing
    /// `lqusers.toml` to disk.
    #[error("Unable to serialize to TOML")]
    SerializationError(toml_edit::ser::Error),
    /// Failed to persist `lqusers.toml` (e.g., permissions or IO).
    #[error("Unable to write lqusers.toml")]
    UnableToWrite,
    /// Failed to read `lqusers.toml` from disk.
    #[error("Unable to read lqusers.toml")]
    UnableToRead,
    /// Failed to parse `lqusers.toml` contents as valid TOML.
    #[error("Unable to parse lqusers.toml")]
    UnableToParse,
    /// Password hash creation or verification failed.
    #[error("Unable to process password hash")]
    PasswordHashError(argon2::password_hash::Error),
    /// Attempted to remove or reference a user that does not exist.
    #[error("User not found")]
    UserNotFound,
    /// Username/password did not match.
    #[error("Invalid Login")]
    InvalidLogin,
}
