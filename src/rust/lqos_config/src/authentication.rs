//! The `authentication` module provides authorization for use of the
//! local web UI on LibreQoS boxes. It maps to `/<install dir>/lqusers.toml`

use log::{error, warn};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
  fmt::Display,
  fs::{read_to_string, remove_file, OpenOptions},
  io::Write,
  path::{Path, PathBuf},
};
use thiserror::Error;
use uuid::Uuid;

/// Access rights of a user
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
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

impl Display for UserRole {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      UserRole::Admin => write!(f, "admin"),
      UserRole::ReadOnly => write!(f, "read-only"),
    }
  }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct WebUser {
  username: String,
  password_hash: String,
  role: UserRole,
  token: String,
}

/// Container holding the authorized web users.
#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct WebUsers {
  allow_unauthenticated_to_view: bool,
  users: Vec<WebUser>,
}

impl WebUsers {
  fn path() -> Result<PathBuf, AuthenticationError> {
    let base_path = crate::EtcLqos::load()
      .map_err(|_| AuthenticationError::UnableToLoadEtcLqos)?
      .lqos_directory;
    let filename = Path::new(&base_path).join("lqusers.toml");
    Ok(filename)
  }

  fn save_to_disk(&self) -> Result<(), AuthenticationError> {
    let path = Self::path()?;
    let new_contents = toml_edit::ser::to_string(&self);
    if let Err(e) = new_contents {
      return Err(AuthenticationError::SerializationError(e));
    }
    let new_contents = new_contents.unwrap();
    if path.exists() && remove_file(&path).is_err() {
      error!("Unable to delete web users file");
      return Err(AuthenticationError::UnableToDelete);
    }
    if let Ok(mut file) =
      OpenOptions::new().write(true).create_new(true).open(path)
    {
      if file.write_all(new_contents.as_bytes()).is_err() {
        error!("Unable to write web users file to disk.");
        return Err(AuthenticationError::UnableToWrite);
      }
    } else {
      error!("Unable to open web users file for writing.");
      return Err(AuthenticationError::UnableToWrite);
    }
    Ok(())
  }

  /// Does the user's file exist? True if it does, false otherwise.
  pub fn does_users_file_exist() -> Result<bool, AuthenticationError> {
    Ok(Self::path()?.exists())
  }

  /// Try to load `lqusers.toml`. If it is unavailable, create a new--empty--
  /// file.
  pub fn load_or_create() -> Result<Self, AuthenticationError> {
    let path = Self::path()?;
    if !path.exists() {
      // Create a new users file, save it and return the
      // empty file
      let new_users = Self::default();
      new_users.save_to_disk()?;
      Ok(new_users)
    } else {
      // Load from disk
      if let Ok(raw) = read_to_string(path) {
        let parse_result = toml_edit::de::from_str(&raw);
        if let Ok(users) = parse_result {
          Ok(users)
        } else {
          error!("Unable to deserialize lqusers.toml. Error in next message.");
          error!("{:?}", parse_result);
          Err(AuthenticationError::UnableToParse)
        }
      } else {
        error!("Unable to read lqusers.toml");
        Err(AuthenticationError::UnableToRead)
      }
    }
  }

  fn hash_password(password: &str) -> String {
    let salted = format!("!x{password}_LibreQosLikesPasswordsForDinner");
    let mut sha256 = Sha256::new();
    sha256.update(salted);
    format!("{:X}", sha256.finalize())
  }

  /// If a user exists with this username, update their details to the
  /// provided values. If the user does not exist, create them with the
  /// provided values.
  pub fn add_or_update_user(
    &mut self,
    username: &str,
    password: &str,
    role: UserRole,
  ) -> Result<String, AuthenticationError> {
    let token; // Assigned in a branch
    if let Some(user) =
      self.users.iter_mut().find(|u| u.username == username)
    {
      user.password_hash = Self::hash_password(password);
      user.role = role;
      token = user.token.clone();
    } else {
      token = Uuid::new_v4().to_string();
      let new_user = WebUser {
        username: username.to_string(),
        password_hash: Self::hash_password(password),
        role,
        token: token.clone(),
      };
      self.users.push(new_user);
    }

    self.save_to_disk()?;
    Ok(token)
  }

  /// Delete a user from `lqusers.toml`
  pub fn remove_user(
    &mut self,
    username: &str,
  ) -> Result<(), AuthenticationError> {
    let old_len = self.users.len();
    self.users.retain(|u| u.username != username);
    if old_len == self.users.len() {
      error!("User {username} not found, hence not deleted.");
      return Err(AuthenticationError::UserNotFound);
    }
    self.save_to_disk()?;
    Ok(())
  }

  /// Attempt a login with the specified username and password. If
  /// the login succeeds, returns the publically shareable token that
  /// uniquely identifies the user a a string. If it fails, returns an
  /// `Err`.
  pub fn login(
    &self,
    username: &str,
    password: &str,
  ) -> Result<String, AuthenticationError> {
    let hash = Self::hash_password(password);
    if let Some(user) = self
      .users
      .iter()
      .find(|u| u.username == username && u.password_hash == hash)
    {
      Ok(user.token.clone())
    } else if self.allow_unauthenticated_to_view {
      Ok("default".to_string())
    } else {
      Err(AuthenticationError::InvalidLogin)
    }
  }

  /// Given a token, lookup the matching user and return their role.
  pub fn get_role_from_token(
    &self,
    token: &str,
  ) -> Result<UserRole, AuthenticationError> {
    if let Some(user) = self.users.iter().find(|u| u.token == token) {
      Ok(user.role)
    } else if self.allow_unauthenticated_to_view {
      Ok(UserRole::ReadOnly)
    } else {
      warn!("Token {token} not found, invalid data access attempt.");
      Err(AuthenticationError::InvalidToken)
    }
  }

  /// Given a token, lookup the matching user and return their username.
  pub fn get_username(&self, token: &str) -> String {
    if let Some(user) = self.users.iter().find(|u| u.token == token) {
      user.username.clone()
    } else {
      "Anonymous".to_string()
    }
  }

  /// Dump all users to the console.
  pub fn print_users(&self) -> Result<(), AuthenticationError> {
    self.users.iter().for_each(|u| {
      println!("{:<40} {:<10}", u.username, u.role.to_string());
    });
    Ok(())
  }

  /// Sets the "allow unauthenticated users" field. If true,
  /// unauthenticated users gain read-only access. This is useful
  /// for demonstration purposes.
  pub fn allow_anonymous(
    &mut self,
    allow: bool,
  ) -> Result<(), AuthenticationError> {
    self.allow_unauthenticated_to_view = allow;
    self.save_to_disk()?;
    Ok(())
  }

  /// Do we allow unauthenticated users to read site data?
  pub fn do_we_allow_anonymous(&self) -> bool {
    self.allow_unauthenticated_to_view
  }
}

#[derive(Error, Debug)]
pub enum AuthenticationError {
  #[error("Unable to load /etc/lqos.conf")]
  UnableToLoadEtcLqos,
  #[error("Unable to serialize to TOML")]
  SerializationError(toml_edit::ser::Error),
  #[error("Unable to remove existing web users file")]
  UnableToDelete,
  #[error("Unable to open lqusers.toml for writing. Check permissions?")]
  UnableToWrite,
  #[error("Unable to read lqusers.toml")]
  UnableToRead,
  #[error("Unable to parse lqusers.toml")]
  UnableToParse,
  #[error("User not found")]
  UserNotFound,
  #[error("Invalid Login")]
  InvalidLogin,
  #[error("Invalid User Token")]
  InvalidToken,
}
