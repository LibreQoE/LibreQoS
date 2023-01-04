use rocket::fs::NamedFile;
use crate::cache_control::{LongCache, NoCache};

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/config")]
pub async fn config_page<'a>() -> NoCache<Option<NamedFile>> {
    NoCache::new(NamedFile::open("static/config.html").await.ok())
}