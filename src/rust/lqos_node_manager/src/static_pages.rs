use rocket::fs::NamedFile;
use crate::cache_control::{LongCache, NoCache};

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/")]
pub async fn index<'a>() -> NoCache<Option<NamedFile>> {
    NoCache::new(NamedFile::open("static/main.html").await.ok())
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/shaped")]
pub async fn shaped_devices_csv_page<'a>() -> NoCache<Option<NamedFile>> {
    NoCache::new(NamedFile::open("static/shaped.html").await.ok())
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/circuit_queue")]
pub async fn circuit_queue<'a>() -> NoCache<Option<NamedFile>> {
    NoCache::new(NamedFile::open("static/circuit_queue.html").await.ok())
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/unknown")]
pub async fn unknown_devices_page<'a>() -> NoCache<Option<NamedFile>> {
    NoCache::new(NamedFile::open("static/unknown-ip.html").await.ok())
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/shaped-add")]
pub async fn shaped_devices_add_page<'a>() -> NoCache<Option<NamedFile>> {
    NoCache::new(NamedFile::open("static/shaped-add.html").await.ok())
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

#[get("/vendor/plotly-2.16.1.min.js")]
pub async fn plotly_js<'a>() -> LongCache<Option<NamedFile>> {
    LongCache::new(NamedFile::open("static/vendor/plotly-2.16.1.min.js").await.ok())
}

#[get("/vendor/jquery.min.js")]
pub async fn jquery_js<'a>() -> LongCache<Option<NamedFile>> {
    LongCache::new(NamedFile::open("static/vendor/jquery.min.js").await.ok())
}

#[get("/vendor/bootstrap.bundle.min.js")]
pub async fn bootsrap_js<'a>() -> LongCache<Option<NamedFile>> {
    LongCache::new(NamedFile::open("static/vendor/bootstrap.bundle.min.js").await.ok())
}

#[get("/vendor/tinylogo.svg")]
pub async fn tinylogo<'a>() -> LongCache<Option<NamedFile>> {
    LongCache::new(NamedFile::open("static/tinylogo.svg").await.ok())
}

#[get("/favicon.ico")]
pub async fn favicon<'a>() -> LongCache<Option<NamedFile>> {
    LongCache::new(NamedFile::open("static/favicon.ico").await.ok())
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