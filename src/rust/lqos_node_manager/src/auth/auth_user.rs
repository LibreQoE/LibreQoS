use axum_login::{
	axum_sessions::{
		async_session::{
			MemoryStore as SessionMemoryStore
		},
		SessionLayer,
	},
	UserStore,
    secrecy::SecretVec,
    AuthLayer, AuthUser, RequireAuthorizationLayer
};

use axum::extract::State;

use async_redis_session::RedisSessionStore;
use std::{collections::HashMap, env, marker::PhantomData, sync::{Arc, Mutex}};
use serde::{Serialize, Deserialize};
use lazy_static::lazy_static;
use rand::Rng;
use tokio::sync::RwLock;
use lqos_config::{UserRole, WebUser, WebUsers};
use core::fmt::Display;

use crate::error::AppError;
use crate::AppState;
use crate::auth::user_store::WebUserStore;
use uuid::Uuid;

lazy_static! {
	pub static ref SECRET: [u8; 64] = (*b"Z0FlIV4wenVQIT54NWJJVHcieyppSSdwOyVMQGNFO3pBVFRAZ3pkMnV2YzwnOHR4").try_into().unwrap();
}

/// Access rights of a user
#[derive(PartialOrd, PartialEq, Clone, Copy, Debug, Deserialize, Serialize)]
pub enum Role {
  Anonymous,
  ReadOnly,
  Admin,
}

impl From<UserRole> for Role {
    fn from(a: UserRole) -> Self {
        let serialised = serde_json::to_value(&a).unwrap();
        serde_json::from_value(serialised).unwrap()
    }
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
	pub password_hash: String,
	pub role: Role,
}

impl From<WebUser> for User {
    fn from(a: WebUser) -> Self {
        Self {
            id: a.token,
            username: a.username,
            password_hash: a.password_hash,
            role: a.role.into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl AuthUser<Role> for User {
    fn get_id(&self) -> String {
        self.id.clone()
    }
    fn get_password_hash(&self) -> SecretVec<u8> {
        SecretVec::new(self.password_hash.clone().into())
    }
    fn get_role(&self) -> Option<Role> {
        Some(self.role.clone())
    }
}

pub type AuthContext = axum_login::extractors::AuthContext<User, WebUserStore<User>, Role>;

pub type RequireAuth = RequireAuthorizationLayer<User, Role>;

pub fn session_layer() -> SessionLayer<RedisSessionStore> {
	let store = RedisSessionStore::new("redis://127.0.0.1/").unwrap();
	SessionLayer::new(store, SECRET.as_ref())
        .with_session_ttl(Some(std::time::Duration::from_secs(60 * 60)))
        .with_secure(false)
}

pub fn auth_layer() -> AuthLayer<WebUserStore<User>, User, Role> {
	let store = WebUserStore::new(WebUsers::load_or_create().unwrap());
	AuthLayer::new(store, SECRET.as_ref())
}

pub async fn authenticate_user(data: Credentials, mut auth: AuthContext) -> Result<bool, AppError> {
	if WebUsers::does_users_file_exist().unwrap() {
		if let Some(users) = Some(WebUsers::load_or_create().unwrap()) {
			if let Ok(token) = users.login(&data.username, &data.password) {
				let user: User = User::from(users.get_user_from_token(&token.as_str()).unwrap());
				match auth.login(&user).await {
					Ok(_) => {
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