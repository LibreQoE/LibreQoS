use serde::{Deserialize, Serialize};

use lqos_config::load_config;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DashletSave {
    pub name: String,
    pub entries: Vec<DashletIdentity>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DashletIdentity {
    pub name: String,
    pub tag: String,
    pub size: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ThemeEntry {
    pub name: String,
    pub path: String,
}

fn dashboards_dir() -> Option<std::path::PathBuf> {
    let config = load_config().ok()?;
    let base_path = std::path::Path::new(&config.lqos_directory)
        .join("bin")
        .join("dashboards");
    if !base_path.exists() {
        if std::fs::create_dir(&base_path).is_err() {
            return None;
        }
    }
    Some(base_path)
}

fn normalize_theme_filename(name: &str) -> String {
    let safe = name.replace('/', "_").replace('\\', "_");
    if safe.ends_with(".json") {
        safe
    } else {
        format!("{}.json", safe)
    }
}

pub(crate) fn list_theme_entries() -> Vec<ThemeEntry> {
    let Some(base_path) = dashboards_dir() else {
        return Vec::new();
    };

    let mut result = Vec::new();
    let entries = match std::fs::read_dir(&base_path) {
        Ok(entries) => entries,
        Err(_) => return result,
    };

    for f in entries.flatten() {
        let fs = f.file_name().to_string_lossy().to_string();
        if !fs.ends_with(".json") {
            continue;
        }
        let mut display_name = fs.trim_end_matches(".json").to_string();
        if let Ok(raw) = std::fs::read_to_string(f.path()) {
            if let Ok(parsed) = serde_json::from_str::<DashletSave>(&raw) {
                if !parsed.name.is_empty() {
                    display_name = parsed.name;
                }
            }
        }
        result.push(ThemeEntry {
            name: display_name,
            path: fs,
        });
    }
    result
}

pub(crate) fn load_theme_entries(name: &str) -> Vec<DashletIdentity> {
    let Some(base_path) = dashboards_dir() else {
        return Vec::new();
    };

    let filename = normalize_theme_filename(name);
    let file_path = base_path.join(filename);
    if !file_path.exists() {
        return Vec::new();
    }

    let raw = match std::fs::read_to_string(&file_path) {
        Ok(raw) => raw,
        Err(_) => return Vec::new(),
    };
    let result: DashletSave = match serde_json::from_str(&raw) {
        Ok(result) => result,
        Err(_) => return Vec::new(),
    };
    result.entries
}

pub(crate) fn save_theme_data(data: &DashletSave) -> Result<(), String> {
    let base_path = dashboards_dir().ok_or_else(|| "Unable to load configuration".to_string())?;
    let filename = normalize_theme_filename(&data.name);
    let file_path = base_path.join(filename);
    let serialized =
        serde_json::to_string(data).map_err(|e| format!("Serialize error: {e}"))?;
    std::fs::write(&file_path, serialized.as_bytes())
        .map_err(|e| format!("Write error: {e}"))?;
    Ok(())
}

pub(crate) fn delete_theme_file(name: &str) -> Result<(), String> {
    let base_path = dashboards_dir().ok_or_else(|| "Unable to load configuration".to_string())?;
    let filename = normalize_theme_filename(name);
    let file_path = base_path.join(filename);
    if file_path.exists() {
        std::fs::remove_file(file_path).map_err(|e| format!("Delete error: {e}"))?;
    }
    Ok(())
}
