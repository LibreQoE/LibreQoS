use anyhow::Result;
use serde::de::DeserializeOwned;

fn url_fixup(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    if base.contains("/nms/api/v2.1") {
        base.to_string()
    } else {
        format!("{base}/nms/api/v2.1")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_fixup_adds_path_when_missing() {
        assert_eq!(
            url_fixup("https://example.com"),
            "https://example.com/nms/api/v2.1"
        );
    }

    #[test]
    fn test_url_fixup_removes_trailing_slash() {
        assert_eq!(
            url_fixup("https://example.com/"),
            "https://example.com/nms/api/v2.1"
        );
    }

    #[test]
    fn test_url_fixup_keeps_existing_path() {
        assert_eq!(
            url_fixup("https://example.com/nms/api/v2.1"),
            "https://example.com/nms/api/v2.1"
        );
    }

    #[test]
    fn test_url_fixup_removes_slash_before_existing_path() {
        assert_eq!(
            url_fixup("https://example.com/nms/api/v2.1/"),
            "https://example.com/nms/api/v2.1"
        );
    }

    #[test]
    fn test_url_fixup_trims_whitespace() {
        assert_eq!(
            url_fixup("   https://example.com   "),
            "https://example.com/nms/api/v2.1"
        );
    }

    #[test]
    fn test_url_fixup_handles_subpath_correctly() {
        assert_eq!(
            url_fixup("https://example.com/nms/api/v2.1/devices"),
            "https://example.com/nms/api/v2.1/devices"
        );
    }

    #[test]
    fn test_url_fixup_removes_multiple_trailing_slashes() {
        assert_eq!(
            url_fixup("https://example.com////"),
            "https://example.com/nms/api/v2.1"
        );
    }
}

/// Submits a request to the UNMS API and returns the result as unprocessed text.
/// This is a debug function: it doesn't do any parsing
#[allow(dead_code)]
pub async fn nms_request_get_text(
    url: &str,
    key: &str,
    api: &str,
) -> Result<String, reqwest::Error> {
    let full_url = format!("{}/{}", url_fixup(api), url);
    //println!("{full_url}");
    let client = reqwest::Client::new();

    let res = client
        .get(&full_url)
        .header("'Content-Type", "application/json")
        .header("X-Auth-Token", key)
        .send()
        .await
        .unwrap();

    res.text().await
}

/// Submits a request to the UNMS API, returning a deserialized vector of type T.
#[allow(dead_code)]
pub async fn nms_request_get_vec<T>(
    url: &str,
    key: &str,
    api: &str,
) -> Result<Vec<T>, reqwest::Error>
where
    T: DeserializeOwned,
{
    let full_url = format!("{}/{}", url_fixup(api), url);
    //println!("{full_url}");
    let client = reqwest::Client::new();

    let res = client
        .get(&full_url)
        .header("'Content-Type", "application/json")
        .header("X-Auth-Token", key)
        .send()
        .await?;

    res.json::<Vec<T>>().await
}

#[allow(dead_code)]
pub async fn nms_request_get_one<T>(url: &str, key: &str, api: &str) -> Result<T, reqwest::Error>
where
    T: DeserializeOwned,
{
    let full_url = format!("{}/{}", url_fixup(api), url);
    //println!("{full_url}");
    let client = reqwest::Client::new();

    let res = client
        .get(&full_url)
        .header("'Content-Type", "application/json")
        .header("X-Auth-Token", key)
        .send()
        .await?;

    res.json::<T>().await
}

/// This is a debug function: it doesn't do any parsing
#[allow(dead_code)]
pub async fn crm_request_get_text(
    api: &str,
    key: &str,
    url: &str,
) -> Result<String, reqwest::Error> {
    let full_url = format!("{}/{}", url_fixup(api), url);
    let client = reqwest::Client::new();

    let res = client
        .get(&full_url)
        .header("'Content-Type", "application/json")
        .header("X-Auth-App-Key", key)
        .send()
        .await?;

    res.text().await
}

#[allow(dead_code)]
pub async fn crm_request_get_vec<T>(
    api: &str,
    key: &str,
    url: &str,
) -> Result<Vec<T>, reqwest::Error>
where
    T: DeserializeOwned,
{
    let full_url = format!("{}/{}", api, url);
    let client = reqwest::Client::new();

    let res = client
        .get(&full_url)
        .header("'Content-Type", "application/json")
        .header("X-Auth-App-Key", key)
        .send()
        .await?;

    res.json::<Vec<T>>().await
}

#[allow(dead_code)]
pub async fn crm_request_get_one<T>(api: &str, key: &str, url: &str) -> Result<T, reqwest::Error>
where
    T: DeserializeOwned,
{
    let full_url = format!("{}/{}", api, url);
    let client = reqwest::Client::new();

    let res = client
        .get(&full_url)
        .header("'Content-Type", "application/json")
        .header("X-Auth-App-Key", key)
        .send()
        .await?;

    res.json::<T>().await
}
