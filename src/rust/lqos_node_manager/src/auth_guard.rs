use std::sync::Mutex;

use anyhow::Error;
use lqos_config::{UserRole, WebUsers};
use once_cell::sync::Lazy;
use rocket::serde::{json::Json, Deserialize, Serialize};
use rocket::{
  http::{Cookie, CookieJar, Status},
  request::{FromRequest, Outcome},
  Request,
};

static WEB_USERS: Lazy<Mutex<Option<WebUsers>>> =
  Lazy::new(|| Mutex::new(None));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthGuard {
  Admin,
  ReadOnly,
  FirstUse,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AuthGuard {
  type Error = anyhow::Error; // Decorated because Error=Error looks odd

  async fn from_request(
    request: &'r Request<'_>,
  ) -> Outcome<Self, Self::Error> {
    let mut lock = WEB_USERS.lock().unwrap();
    if lock.is_none() {
      if WebUsers::does_users_file_exist().unwrap() {
        *lock = Some(WebUsers::load_or_create().unwrap());
      } else {
        // There is no user list, so we're redirecting to the
        // new user page.
        return Outcome::Success(AuthGuard::FirstUse);
      }
    }

    if let Some(users) = &*lock {
      if let Some(token) = request.cookies().get("User-Token") {
        match users.get_role_from_token(token.value()) {
          Ok(UserRole::Admin) => return Outcome::Success(AuthGuard::Admin),
          Ok(UserRole::ReadOnly) => {
            return Outcome::Success(AuthGuard::ReadOnly)
          }
          _ => {
            return Outcome::Error((
              Status::Unauthorized,
              Error::msg("Invalid token"),
            ))
          }
        }
      } else {
        // If no login, do we allow anonymous?
        if users.do_we_allow_anonymous() {
          return Outcome::Success(AuthGuard::ReadOnly);
        }
      }
    }

    Outcome::Error((Status::Unauthorized, Error::msg("Access Denied")))
  }
}

impl AuthGuard {}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "rocket::serde")]
pub struct FirstUser {
  pub allow_anonymous: bool,
  pub username: String,
  pub password: String,
}

#[post("/api/create_first_user", data = "<info>")]
pub fn create_first_user(
  cookies: &CookieJar,
  info: Json<FirstUser>,
) -> Json<String> {
  if WebUsers::does_users_file_exist().unwrap() {
    return Json("ERROR".to_string());
  }
  let mut lock = WEB_USERS.lock().unwrap();
  let mut users = WebUsers::load_or_create().unwrap();
  users.allow_anonymous(info.allow_anonymous).unwrap();
  let token = users
    .add_or_update_user(&info.username, &info.password, UserRole::Admin)
    .unwrap();
  cookies.add(Cookie::new("User-Token", token));
  *lock = Some(users);
  Json("OK".to_string())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "rocket::serde")]
pub struct LoginAttempt {
  pub username: String,
  pub password: String,
}

#[post("/api/login", data = "<info>")]
pub fn login(cookies: &CookieJar, info: Json<LoginAttempt>) -> Json<String> {
  let mut lock = WEB_USERS.lock().unwrap();
  if lock.is_none() && WebUsers::does_users_file_exist().unwrap() {
    *lock = Some(WebUsers::load_or_create().unwrap());
  }
  if let Some(users) = &*lock {
    if let Ok(token) = users.login(&info.username, &info.password) {
      cookies.add(Cookie::new("User-Token", token));
      return Json("OK".to_string());
    }
  }
  Json("ERROR".to_string())
}

#[get("/api/admin_check")]
pub fn admin_check(auth: AuthGuard) -> Json<bool> {
  match auth {
    AuthGuard::Admin => Json(true),
    _ => Json(false),
  }
}

#[get("/api/username")]
pub fn username(_auth: AuthGuard, cookies: &CookieJar) -> Json<String> {
  if let Some(token) = cookies.get("User-Token") {
    let lock = WEB_USERS.lock().unwrap();
    if let Some(users) = &*lock {
      return Json(users.get_username(token.value()));
    }
  }
  Json("Anonymous".to_string())
}
