use crate::{
  auth_guard::AuthGuard,
  cache_control::{LongCache, NoCache},
};
use rocket::fs::NamedFile;

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/")]
pub async fn index<'a>(auth: AuthGuard) -> NoCache<Option<NamedFile>> {
  match auth {
    AuthGuard::FirstUse => {
      NoCache::new(NamedFile::open("static/first_run.html").await.ok())
    }
    _ => NoCache::new(NamedFile::open("static/main.html").await.ok()),
  }
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[catch(401)]
pub async fn login<'a>() -> NoCache<Option<NamedFile>> {
  NoCache::new(NamedFile::open("static/login.html").await.ok())
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/login")]
pub async fn login_page<'a>() -> NoCache<Option<NamedFile>> {
  NoCache::new(NamedFile::open("static/login.html").await.ok())
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/shaped")]
pub async fn shaped_devices_csv_page<'a>(
  _auth: AuthGuard,
) -> NoCache<Option<NamedFile>> {
  NoCache::new(NamedFile::open("static/shaped.html").await.ok())
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/circuit_queue")]
pub async fn circuit_queue<'a>(
  _auth: AuthGuard,
) -> NoCache<Option<NamedFile>> {
  NoCache::new(NamedFile::open("static/circuit_queue.html").await.ok())
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/ip_dump")]
pub async fn ip_dump<'a>(
  _auth: AuthGuard,
) -> NoCache<Option<NamedFile>> {
  NoCache::new(NamedFile::open("static/ip_dump.html").await.ok())
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/unknown")]
pub async fn unknown_devices_page<'a>(
  _auth: AuthGuard,
) -> NoCache<Option<NamedFile>> {
  NoCache::new(NamedFile::open("static/unknown-ip.html").await.ok())
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/shaped-add")]
pub async fn shaped_devices_add_page<'a>(
  _auth: AuthGuard,
) -> NoCache<Option<NamedFile>> {
  NoCache::new(NamedFile::open("static/shaped-add.html").await.ok())
}

// Temporary for funsies
#[get("/showoff")]
pub async fn pretty_map_graph<'a>(
  _auth: AuthGuard,
) -> NoCache<Option<NamedFile>> {
  NoCache::new(NamedFile::open("static/showoff.html").await.ok())
}

// Help me obi-wan, you're our only hope
#[get("/help")]
pub async fn help_page<'a>(
  _auth: AuthGuard,
) -> NoCache<Option<NamedFile>> {
  NoCache::new(NamedFile::open("static/help.html").await.ok())
}

#[get("/vendor/bootstrap.min.css")]
pub async fn bootsrap_css<'a>() -> LongCache<Option<NamedFile>> {
  LongCache::new(NamedFile::open("static/vendor/bootstrap.min.css").await.ok())
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/lqos.js")]
pub async fn lqos_js<'a>() -> NoCache<Option<NamedFile>> {
  NoCache::new(NamedFile::open("static/lqos.js").await.ok())
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/lqos.css")]
pub async fn lqos_css<'a>() -> NoCache<Option<NamedFile>> {
  NoCache::new(NamedFile::open("static/lqos.css").await.ok())
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/vendor/klingon.ttf")]
pub async fn klingon<'a>() -> LongCache<Option<NamedFile>> {
  LongCache::new(NamedFile::open("static/vendor/klingon.ttf").await.ok())
}

#[get("/vendor/plotly-2.16.1.min.js")]
pub async fn plotly_js<'a>() -> LongCache<Option<NamedFile>> {
  LongCache::new(
    NamedFile::open("static/vendor/plotly-2.16.1.min.js").await.ok(),
  )
}

#[get("/vendor/jquery.min.js")]
pub async fn jquery_js<'a>() -> LongCache<Option<NamedFile>> {
  LongCache::new(NamedFile::open("static/vendor/jquery.min.js").await.ok())
}

#[get("/vendor/msgpack.min.js")]
pub async fn msgpack_js<'a>() -> LongCache<Option<NamedFile>> {
  LongCache::new(NamedFile::open("static/vendor/msgpack.min.js").await.ok())
}

#[get("/vendor/bootstrap.bundle.min.js")]
pub async fn bootsrap_js<'a>() -> LongCache<Option<NamedFile>> {
  LongCache::new(
    NamedFile::open("static/vendor/bootstrap.bundle.min.js").await.ok(),
  )
}

#[get("/vendor/tinylogo.svg")]
pub async fn tinylogo<'a>() -> LongCache<Option<NamedFile>> {
  LongCache::new(NamedFile::open("static/tinylogo.svg").await.ok())
}

#[get("/favicon.png")]
pub async fn favicon<'a>() -> LongCache<Option<NamedFile>> {
  LongCache::new(NamedFile::open("static/favicon.png").await.ok())
}

/// FontAwesome icons
#[get("/vendor/solid.min.css")]
pub async fn fontawesome_solid<'a>() -> LongCache<Option<NamedFile>> {
  LongCache::new(NamedFile::open("static/vendor/solid.min.css").await.ok())
}

#[get("/fonts/fontawesome-webfont.ttf")]
pub async fn fontawesome_webfont<'a>() -> LongCache<Option<NamedFile>> {
  LongCache::new(NamedFile::open("static/vendor/fa-webfont.ttf").await.ok())
}

#[get("/fonts/fontawesome-webfont.woff2")]
pub async fn fontawesome_woff<'a>() -> LongCache<Option<NamedFile>> {
  LongCache::new(NamedFile::open("static/vendor/fa-webfont.ttf").await.ok())
}
