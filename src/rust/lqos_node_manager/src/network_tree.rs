use lqos_bus::{bus_request, BusRequest, BusResponse};
use lqos_config::NetworkJsonNode;
use rocket::{fs::NamedFile, serde::json::Json};

use crate::cache_control::NoCache;

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/tree")]
pub async fn tree_page<'a>() -> NoCache<Option<NamedFile>> {
  NoCache::new(NamedFile::open("static/tree.html").await.ok())
}

#[get("/api/network_tree/<parent>")]
pub async fn tree_entry(
  parent: usize,
) -> NoCache<Json<Vec<(usize, NetworkJsonNode)>>> {
  let responses =
    bus_request(vec![BusRequest::GetNetworkMap { parent }]).await.unwrap();
  let result = match &responses[0] {
    BusResponse::NetworkMap(nodes) => nodes.to_owned(),
    _ => Vec::new(),
  };

  NoCache::new(Json(result))
}
