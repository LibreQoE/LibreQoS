use anyhow::{Context, Result};
use bytes::Bytes;
use reqwest::{Method, StatusCode};

#[derive(Clone)]
pub struct ApiClient {
    base: String,
    api_key: String,
    client: reqwest::Client,
}

pub struct ApiResponse {
    pub status: StatusCode,
    pub headers: reqwest::header::HeaderMap,
    pub body: Bytes,
}

impl ApiClient {
    pub fn new(base: String, api_key: String, timeout_secs: u64) -> Result<Self> {
        let base = base.trim_end_matches('/').to_string();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .context("build reqwest client")?;
        Ok(Self {
            base,
            api_key,
            client,
        })
    }

    pub async fn request(
        &self,
        method: Method,
        path_and_query: &str,
        body: Option<reqwest::Body>,
        content_type: Option<&str>,
    ) -> Result<ApiResponse> {
        let url = format!("{}{}", self.base, path_and_query);

        let mut req = self
            .client
            .request(method, &url)
            .header("x-bearer", &self.api_key);
        if let Some(ct) = content_type {
            req = req.header(reqwest::header::CONTENT_TYPE, ct);
        }
        if let Some(body) = body {
            req = req.body(body);
        }

        let resp = req.send().await.context("send request")?;
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = resp.bytes().await.context("read response body")?;

        Ok(ApiResponse {
            status,
            headers,
            body,
        })
    }

    pub async fn get(&self, path_and_query: &str) -> Result<ApiResponse> {
        self.request(Method::GET, path_and_query, None, None).await
    }

    pub async fn post_empty(&self, path_and_query: &str) -> Result<ApiResponse> {
        self.request(Method::POST, path_and_query, None, None).await
    }

    pub async fn delete(&self, path_and_query: &str) -> Result<ApiResponse> {
        self.request(Method::DELETE, path_and_query, None, None)
            .await
    }

    pub async fn post_json(
        &self,
        path_and_query: &str,
        body: serde_json::Value,
    ) -> Result<ApiResponse> {
        self.request(
            Method::POST,
            path_and_query,
            Some(reqwest::Body::from(body.to_string())),
            Some("application/json"),
        )
        .await
    }

    pub async fn put_json(
        &self,
        path_and_query: &str,
        body: serde_json::Value,
    ) -> Result<ApiResponse> {
        self.request(
            Method::PUT,
            path_and_query,
            Some(reqwest::Body::from(body.to_string())),
            Some("application/json"),
        )
        .await
    }

    pub async fn post_text(&self, path_and_query: &str, body: String) -> Result<ApiResponse> {
        self.request(
            Method::POST,
            path_and_query,
            Some(reqwest::Body::from(body)),
            Some("text/plain"),
        )
        .await
    }
}
