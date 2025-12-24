use axum::Json;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use lqos_config::load_config;

pub async fn list_themes() -> Json<Vec<String>> {
    if let Ok(config) = load_config() {
        let base_path = std::path::Path::new(&config.lqos_directory)
            .join("bin")
            .join("dashboards");
        if !base_path.exists() {
            std::fs::create_dir(&base_path).expect("Unable to create dashboards directory");
        }

        let mut result = Vec::new();
        for f in std::fs::read_dir(&base_path).expect("Unable to read dashboards directory") {
            if let Ok(f) = f {
                let fs = f.file_name().to_string_lossy().to_string();
                if fs.ends_with("json") {
                    result.push(fs.to_string());
                }
            }
        }
        return Json(result);
    }
    Json(Vec::new())
}

#[derive(Serialize, Deserialize)]
pub struct DashletSave {
    name: String,
    entries: Vec<DashletIdentity>,
}

#[derive(Serialize, Deserialize)]
pub struct DashletIdentity {
    name: String,
    tag: String,
    size: i32,
}

pub async fn save_theme(Json(data): Json<DashletSave>) -> StatusCode {
    if let Ok(config) = load_config() {
        let base_path = std::path::Path::new(&config.lqos_directory)
            .join("bin")
            .join("dashboards");
        if !base_path.exists() {
            std::fs::create_dir(&base_path).expect("Unable to create dashboards directory");
        }

        let name = data.name.replace('/', "_");
        let name = format!("{}.json", name);
        let file_path = base_path.join(name);
        let serialized = serde_json::to_string(&data).expect("Unable to serialize theme payload");
        std::fs::write(&file_path, serialized.as_bytes()).expect("Unable to write theme file");
    }

    StatusCode::OK
}

#[derive(Deserialize)]
pub struct ThemeSelector {
    theme: String,
}

pub async fn delete_theme(Json(f): Json<ThemeSelector>) -> StatusCode {
    if let Ok(config) = load_config() {
        let base_path = std::path::Path::new(&config.lqos_directory)
            .join("bin")
            .join("dashboards")
            .join(&f.theme);
        if base_path.exists() {
            std::fs::remove_file(base_path).expect("Unable to remove theme file");
        }
    }

    StatusCode::OK
}

pub async fn get_theme(Json(f): Json<ThemeSelector>) -> Json<Vec<DashletIdentity>> {
    if let Ok(config) = load_config() {
        let base_path = std::path::Path::new(&config.lqos_directory)
            .join("bin")
            .join("dashboards")
            .join(&f.theme);
        if base_path.exists() {
            let raw = std::fs::read_to_string(&base_path).expect("Unable to read theme file");
            let result: DashletSave =
                serde_json::from_str(&raw).expect("Unable to parse theme file");
            return Json(result.entries);
        }
    }
    Json(Vec::new())
}
