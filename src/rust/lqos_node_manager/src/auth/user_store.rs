use std::{collections::HashMap, hash::Hash, marker::PhantomData};

use axum_login::{UserStore, AuthUser};

use super::auth_user::*;

use lqos_config::{EtcLqos, LibreQoSConfig, NetworkJsonTransport, ShapedDevice, Tunables, WebUser, WebUsers};

use async_trait::async_trait;

#[derive(Clone, Debug)]
pub struct WebUserStore<User> {
    pub allow_anonymous: bool,
    pub users: HashMap<String, User>,
    _user_type: PhantomData<User>,
    _role_type: PhantomData<Role>,
}

impl WebUserStore<User> {
    pub fn new(web_users: WebUsers) -> Self {
        tracing::debug!("building user store");
        let mut user_list = HashMap::<String, User>::new();
        for web_user in web_users.users.iter() {
            user_list.insert(web_user.token.clone(), User::from(web_user.clone()));
        }
        Self {
            allow_anonymous: web_users.allow_unauthenticated_to_view,
            users: user_list,
            _user_type: Default::default(),
            _role_type: Default::default(),
        }
    }
}

#[async_trait]
impl<Role> UserStore<Role> for WebUserStore<User>
where
    Role: PartialOrd + PartialEq + Clone + Send + Sync + 'static,
    User: AuthUser<Role>,
{
    type User = User;

    async fn load_user(&self, user_id: &str) -> Result<Option<Self::User>, eyre::Report> {
        let user: Option<User> = self.users.get(user_id).cloned();
        match user {
            Some(u) => {
                Ok(Some(u))
            },
            None => {
                Err(eyre::eyre!("Could not find user by user_id: {:?}", user_id))
            }
        }
    }
}