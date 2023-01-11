use anyhow::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt::Display,
    fs::{read_to_string, remove_file, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum UserRole {
    ReadOnly,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WebUsers {
    allow_unauthenticated_to_view: bool,
    users: Vec<WebUser>,
}

impl Default for WebUsers {
    fn default() -> Self {
        Self {
            users: Vec::new(),
            allow_unauthenticated_to_view: false,
        }
    }
}

impl WebUsers {
    fn path() -> Result<PathBuf> {
        let base_path = crate::EtcLqos::load()?.lqos_directory;
        let filename = Path::new(&base_path).join("webusers.toml");
        Ok(filename)
    }

    fn save_to_disk(&self) -> Result<()> {
        let path = Self::path()?;
        let new_contents = toml::to_string(&self)?;
        if path.exists() {
            remove_file(&path)?;
        }
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(&new_contents.as_bytes())?;
        Ok(())
    }

    pub fn does_users_file_exist() -> Result<bool> {
        Ok(Self::path()?.exists())
    }

    pub fn load_or_create() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            // Create a new users file, save it and return the
            // empty file
            let new_users = Self::default();
            new_users.save_to_disk()?;
            Ok(new_users)
        } else {
            // Load from disk
            let raw = read_to_string(path)?;
            let users = toml::from_str(&raw)?;
            Ok(users)
        }
    }

    fn hash_password(password: &str) -> String {
        let salted = format!("!x{password}_LibreQosLikesPasswordsForDinner");
        let mut sha256 = Sha256::new();
        sha256.update(salted);
        format!("{:X}", sha256.finalize())
    }

    pub fn add_or_update_user(
        &mut self,
        username: &str,
        password: &str,
        role: UserRole,
    ) -> Result<String> {
        let token; // Assigned in a branch
        if let Some(mut user) = self.users.iter_mut().find(|u| u.username == username) {
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

    pub fn remove_user(&mut self, username: &str) -> Result<()> {
        let old_len = self.users.len();
        self.users.retain(|u| u.username != username);
        if old_len == self.users.len() {
            return Err(Error::msg(format!("User {} was not found", username)));
        }
        self.save_to_disk()?;
        Ok(())
    }

    pub fn login(&self, username: &str, password: &str) -> Result<String> {
        let hash = Self::hash_password(password);
        if let Some(user) = self
            .users
            .iter()
            .find(|u| u.username == username && u.password_hash == hash)
        {
            Ok(user.token.clone())
        } else {
            if self.allow_unauthenticated_to_view {
                Ok("default".to_string())
            } else {
                Err(Error::msg("Invalid Login"))
            }
        }
    }

    pub fn get_role_from_token(&self, token: &str) -> Result<UserRole> {
        if let Some(user) = self.users.iter().find(|u| u.token == token) {
            Ok(user.role)
        } else {
            if self.allow_unauthenticated_to_view {
                Ok(UserRole::ReadOnly)
            } else {
                Err(Error::msg("Unknown user token"))
            }
        }
    }

    pub fn get_username(&self, token: &str) -> String {
        if let Some(user) = self.users.iter().find(|u| u.token == token) {
            user.username.clone()
        } else {
            "Anonymous".to_string()
        }
    }

    pub fn print_users(&self) -> Result<()> {
        self.users.iter().for_each(|u| {
            println!("{:<40} {:<10}", u.username, u.role.to_string());
        });
        Ok(())
    }

    pub fn allow_anonymous(&mut self, allow: bool) -> Result<()> {
        self.allow_unauthenticated_to_view = allow;
        self.save_to_disk()?;
        Ok(())
    }

    pub fn do_we_allow_anonymous(&self) -> bool {
        self.allow_unauthenticated_to_view
    }
}
