use rocket::http::Header;
use rocket::response::Responder;

/// Use to wrap a responder when you want to tell the user's
/// browser to try and cache a response.
/// 
/// For example:
/// 
/// ```
/// pub async fn bootsrap_css<'a>() -> LongCache<Option<NamedFile>> {
///     LongCache::new(NamedFile::open("static/vendor/bootstrap.min.css").await.ok())
/// }
/// ```
#[derive(Responder)]
pub struct LongCache<T> {
    inner: T,
    my_header: Header<'static>,
}
impl<'r, 'o: 'r, T: Responder<'r, 'o>> LongCache<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            my_header: Header::new("cache-control", "max-age=604800, public"),
        }
    }
}

/// Use to wrap a responder when you want to tell the user's
/// browser to keep data private and never cahce it.
/// 
/// For example:
/// 
/// ```
/// pub async fn bootsrap_css<'a>() -> LongCache<Option<NamedFile>> {
///     LongCache::new(NamedFile::open("static/vendor/bootstrap.min.css").await.ok())
/// }
/// ```
#[derive(Responder)]
pub struct NoCache<T> {
    inner: T,
    my_header: Header<'static>,
}
impl<'r, 'o: 'r, T: Responder<'r, 'o>> NoCache<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            my_header: Header::new("cache-control", "no-cache, private"),
        }
    }
}