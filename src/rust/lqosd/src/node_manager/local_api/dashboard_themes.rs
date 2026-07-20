use serde::{Deserialize, Serialize};

use lqos_config::load_config;

const DASHBOARD_THEME_NAME_MAX_CHARS: usize = 64;

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
    if !base_path.exists() && std::fs::create_dir(&base_path).is_err() {
        return None;
    }
    Some(base_path)
}

fn theme_name_stem(name: &str) -> &str {
    let trimmed = name.trim();
    trimmed.strip_suffix(".json").unwrap_or(trimmed)
}

fn is_allowed_theme_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '.' | '_' | '-' | '(' | ')' | '[' | ']')
}

fn validate_theme_name(name: &str) -> Result<String, String> {
    let stem = theme_name_stem(name);
    if stem.is_empty() {
        return Err("Dashboard layout name is required.".to_string());
    }
    if stem.chars().count() > DASHBOARD_THEME_NAME_MAX_CHARS {
        return Err(format!(
            "Dashboard layout names must be {DASHBOARD_THEME_NAME_MAX_CHARS} characters or fewer."
        ));
    }
    if !stem.chars().all(is_allowed_theme_name_char) {
        return Err(
            "Dashboard layout names may only use letters, numbers, spaces, dots, underscores, hyphens, parentheses, and brackets."
                .to_string(),
        );
    }
    Ok(stem.to_string())
}

fn theme_filename(name: &str) -> Result<String, String> {
    let safe_name = validate_theme_name(name)?;
    Ok(format!("{safe_name}.json"))
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
        let Some(fs) = f.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(stem) = fs.strip_suffix(".json") else {
            continue;
        };
        let Ok(mut display_name) = validate_theme_name(stem) else {
            continue;
        };
        if let Ok(raw) = std::fs::read_to_string(f.path())
            && let Ok(parsed) = serde_json::from_str::<DashletSave>(&raw)
            && !parsed.name.is_empty()
            && let Ok(valid_name) = validate_theme_name(&parsed.name)
        {
            display_name = valid_name;
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

    let Ok(filename) = theme_filename(name) else {
        return Vec::new();
    };
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
    let valid_name = validate_theme_name(&data.name)?;
    let filename = theme_filename(&valid_name)?;
    let file_path = base_path.join(filename);
    let sanitized = DashletSave {
        name: valid_name,
        entries: data.entries.clone(),
    };
    let serialized =
        serde_json::to_string(&sanitized).map_err(|e| format!("Serialize error: {e}"))?;
    std::fs::write(&file_path, serialized.as_bytes()).map_err(|e| format!("Write error: {e}"))?;
    Ok(())
}

pub(crate) fn delete_theme_file(name: &str) -> Result<(), String> {
    let base_path = dashboards_dir().ok_or_else(|| "Unable to load configuration".to_string())?;
    let filename = theme_filename(name)?;
    let file_path = base_path.join(filename);
    if file_path.exists() {
        std::fs::remove_file(file_path).map_err(|e| format!("Delete error: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{theme_filename, validate_theme_name};

    #[test]
    fn dashboard_theme_names_allow_operator_friendly_labels() {
        assert_eq!(
            validate_theme_name(" Evening Layout 1 [NOC] ").expect("valid layout name"),
            "Evening Layout 1 [NOC]"
        );
        assert_eq!(
            theme_filename("Primary-Dashboard.json").expect("valid filename"),
            "Primary-Dashboard.json"
        );
    }

    #[test]
    fn dashboard_theme_names_reject_paths_and_markup() {
        assert!(validate_theme_name("").is_err());
        assert!(validate_theme_name("   ").is_err());
        assert!(validate_theme_name("../operator").is_err());
        assert!(validate_theme_name("operator/layout").is_err());
        assert!(validate_theme_name("<img src=x onerror=alert(1)>").is_err());
        assert!(validate_theme_name("layout & alert").is_err());
    }

    #[test]
    fn dashboard_theme_names_enforce_length_limit() {
        assert!(validate_theme_name(&"a".repeat(64)).is_ok());
        assert!(validate_theme_name(&"a".repeat(65)).is_err());
    }
}
