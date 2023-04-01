use axum_login::{
	axum_sessions::{
		async_session::{
			MemoryStore as SessionMemoryStore
		},
		SessionLayer,
	},
	memory_store::MemoryStore as AuthMemoryStore,
    secrecy::SecretVec,
    AuthLayer, AuthUser, RequireAuthorizationLayer
};

use std::{collections::HashMap, sync::Arc};
use serde::{Serialize, Deserialize};
use lazy_static::lazy_static;
use rand::Rng;
use tokio::sync::RwLock;
use lqos_config::WebUsers;
use core::fmt::Display;

use crate::error::AppError;

lazy_static! {
	pub static ref DATABASE: Arc<RwLock<HashMap<String, User>>> = Arc::new(RwLock::new(HashMap::new()));
	pub static ref SECRET: [u8; 64] = rand::thread_rng().gen::<[u8; 64]>();
}

/// Access rights of a user
#[derive(PartialOrd, PartialEq, Clone, Copy, Debug, Deserialize, Serialize)]
pub enum Role {
  Anonymous,
  ReadOnly,
  Admin,
}

impl From<&str> for Role {
  fn from(s: &str) -> Self {
    let s = s.to_lowercase();
    if s == "admin" {
      Role::Admin
    } else if s == "read-only" {
      Role::ReadOnly
    } else {
      Role::Anonymous
    }
  }
}

impl Display for Role {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Role::Admin => write!(f, "admin"),
      Role::ReadOnly => write!(f, "read-only"),
      Role::Anonymous => write!(f, "anonymous"),
    }
  }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct User {
	pub id: String,
	pub username: String,
	password_hash: String,
	pub role: Role,
	pub mode: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl AuthUser<Role> for User {
    fn get_id(&self) -> String {
        format!("{}", self.id)
    }

    fn get_password_hash(&self) -> SecretVec<u8> {
        SecretVec::new(self.password_hash.clone().into())
    }

    fn get_role(&self) -> Option<Role> {
        Some(self.role.clone())
    }
}

pub type AuthContext = axum_login::extractors::AuthContext<User, AuthMemoryStore<User>, Role>;

pub type RequireAuth = RequireAuthorizationLayer<User, Role>;

pub fn session_layer() -> SessionLayer<SessionMemoryStore> {
	let store = SessionMemoryStore::new();
	SessionLayer::new(store, SECRET.as_ref()).with_secure(false)
}

pub fn auth_layer() -> AuthLayer<AuthMemoryStore<User>, User, Role> {
	let store = AuthMemoryStore::new(&DATABASE);
	AuthLayer::new(store, SECRET.as_ref())
}

pub async fn authenticate_user(data: Credentials, mut auth: AuthContext) -> Result<bool, AppError> {
	if WebUsers::does_users_file_exist().unwrap() {
		if let Some(users) = Some(WebUsers::load_or_create().unwrap()) {
			if let Ok(token) = users.login(&data.username, &data.password) {
				let role = users.get_role_from_token(&token.as_str()).unwrap();
				let mut user = User {
					id: token,
					password_hash: "$argon2id$v=19$m=4096,t=3,p=1$L0MVanZGzDvqdp+3uJiHDg$d0R/Bac3IXudaqTIp4d4wBJaSCghXkcuU6ESy1c0JVc".into(),
					role: Role::from(role.to_string().as_str()),
					username: data.username,
					mode: "light".to_string(),
				};
				if user.get_id() == "default" {
					user.role = Role::Anonymous;
					user.username = "Anonymous".to_string();
				}
				// Send user to axum_login for session creation/management
				match auth.login(&user).await {
					Ok(_) => {
						// Write user to memory store
						DATABASE.write().await.insert(user.get_id(), user.clone());
						return Ok(true)
					},
					Err(_) => {
						return Err(AppError::InvalidCredentials)
					}
				}
			}
		}
	}
	Ok(false)
}