use axum::extract::ws::{WebSocket, Message};
use pgdb::sqlx::{Pool, Postgres};
use serde::Serialize;

pub async fn omnisearch(
    cnn: Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    term: &str,
) -> anyhow::Result<()> {
    log::warn!("Searching for {term}");

    let hits = search_devices(cnn.clone(), key, term).await;
    if let Err(e) = &hits {
        log::error!("{e:?}");
    }
    let mut hits = hits.unwrap();

    hits.sort_by(|a,b| a.score.partial_cmp(&b.score).unwrap());

    let msg = SearchMessage {
        msg: "search".to_string(),
        hits
    };
    log::warn!("{msg:?}");
    let json = serde_json::to_string(&msg).unwrap();
    socket.send(Message::Text(json)).await.unwrap();

    Ok(())
}

#[derive(Serialize, Debug)]
struct SearchMessage {
    msg: String,
    hits: Vec<SearchResult>,
}

#[derive(Serialize, Debug)]
struct SearchResult {
    name: String,
    url: String,
    score: f64,
    icon: String,
}

async fn search_devices(
    cnn: Pool<Postgres>,
    key: &str,
    term: &str,
) -> anyhow::Result<Vec<SearchResult>> {
    let hits = pgdb::search_devices(cnn, key, term).await?;
    Ok(hits
        .iter()
        .map(|hit| SearchResult {
            name: hit.circuit_name.to_string(),
            url: format!("circuit:{}", hit.circuit_id),
            score: hit.score,
            icon: "circuit".to_string(),
        })
        .collect())
}