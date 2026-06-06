//! Desktop settings service - local, non-secret configuration persistence.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::types::{CommandResult, DesktopError};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub values: HashMap<String, SettingValue>,
}

impl AppSettings {
    fn load_or_default(path: &PathBuf) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to parse settings at {path:?}: {e}");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    fn save(&self, path: &PathBuf) -> Result<(), DesktopError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| DesktopError::Settings {
                message: format!("failed to create settings dir: {e}"),
            })?;
        }
        let content = serde_json::to_string_pretty(self).map_err(|e| DesktopError::Settings {
            message: format!("failed to serialize settings: {e}"),
        })?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &content).map_err(|e| DesktopError::Settings {
            message: format!("failed to write settings: {e}"),
        })?;
        fs::rename(&tmp, path).map_err(|e| DesktopError::Settings {
            message: format!("failed to persist settings: {e}"),
        })?;
        Ok(())
    }
}

pub struct SettingsManager {
    settings: Mutex<AppSettings>,
    path: PathBuf,
}

impl SettingsManager {
    pub fn new() -> Result<Self, DesktopError> {
        let path = settings_path().map_err(|e| DesktopError::Settings {
            message: format!("failed to resolve settings path: {e}"),
        })?;
        let settings = Mutex::new(AppSettings::load_or_default(&path));
        Ok(Self { settings, path })
    }

    #[cfg(test)]
    pub fn with_path(path: &std::path::Path) -> Result<Self, DesktopError> {
        let settings = Mutex::new(AppSettings::load_or_default(&path.to_path_buf()));
        Ok(Self {
            settings,
            path: path.to_path_buf(),
        })
    }

    pub fn get(&self, key: &str) -> Option<SettingValue> {
        self.settings.lock().unwrap().values.get(key).cloned()
    }

    pub fn set(&self, key: &str, value: SettingValue) -> Result<(), DesktopError> {
        {
            let mut s = self.settings.lock().unwrap();
            s.values.insert(key.to_string(), value);
        }
        self.save()
    }

    fn save(&self) -> Result<(), DesktopError> {
        let s = self.settings.lock().unwrap();
        s.save(&self.path)
    }
}

fn global_manager() -> &'static SettingsManager {
    use std::sync::OnceLock;
    static M: OnceLock<SettingsManager> = OnceLock::new();
    M.get_or_init(|| {
        SettingsManager::new().unwrap_or_else(|e| {
            eprintln!("Warning: settings unavailable: {e}");
            // Safe fallback: use a temp directory file instead of /dev/null
            // to avoid overwriting character devices via atomic rename
            let fallback_path = std::env::temp_dir().join("opensymphony-settings-fallback.json");
            SettingsManager {
                settings: Mutex::new(AppSettings::default()),
                path: fallback_path,
            }
        })
    })
}

fn settings_path() -> Result<PathBuf, String> {
    let proj = directories::ProjectDirs::from("dev", "opensymphony", "app")
        .ok_or("could not determine project directories")?;
    Ok(proj.config_dir().join("settings.json"))
}

#[derive(Debug, Deserialize)]
pub struct GetSettingRequest {
    pub key: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(tag = "type", content = "value")]
pub enum SettingValue {
    Text(String),
    Flag(bool),
    Number(f64),
}

#[derive(Debug, Serialize)]
pub struct GetSettingResponse {
    pub value: Option<SettingValue>,
}

#[derive(Debug, Deserialize)]
pub struct SetSettingRequest {
    pub key: String,
    pub value: SettingValue,
}

#[derive(Debug, Serialize)]
pub struct SetSettingResponse {
    pub persisted: bool,
}

#[tauri::command]
pub async fn get_setting(req: GetSettingRequest) -> CommandResult<GetSettingResponse> {
    let mgr = global_manager();
    Ok(GetSettingResponse {
        value: mgr.get(&req.key),
    })
}

#[tauri::command]
pub async fn set_setting(req: SetSettingRequest) -> CommandResult<SetSettingResponse> {
    let mgr = global_manager();
    mgr.set(&req.key, req.value)?;
    Ok(SetSettingResponse { persisted: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setting_value_serialization() {
        let t = SettingValue::Text("hello".into());
        let j = serde_json::to_string(&t).unwrap();
        assert!(j.contains("Text"));
        let t = SettingValue::Number(42.0);
        let j = serde_json::to_string(&t).unwrap();
        assert!(j.contains("42"));
        let t = SettingValue::Flag(true);
        let j = serde_json::to_string(&t).unwrap();
        assert!(j.contains("true"));
    }

    #[test]
    fn test_settings_manager_round_trip() {
        let tmp = std::env::temp_dir().join(format!("settings_test_{}.json", std::process::id()));
        let mgr = SettingsManager::with_path(&tmp).unwrap();

        mgr.set("test_key", SettingValue::Text("test_value".into()))
            .unwrap();
        assert_eq!(
            mgr.get("test_key"),
            Some(SettingValue::Text("test_value".into()))
        );

        mgr.set("number_key", SettingValue::Number(123.0)).unwrap();
        assert_eq!(mgr.get("number_key"), Some(SettingValue::Number(123.0)));

        mgr.set("flag_key", SettingValue::Flag(true)).unwrap();
        assert_eq!(mgr.get("flag_key"), Some(SettingValue::Flag(true)));

        // Verify persistence by loading from file
        let mgr2 = SettingsManager::with_path(&tmp).unwrap();
        assert_eq!(
            mgr2.get("test_key"),
            Some(SettingValue::Text("test_value".into()))
        );
        assert_eq!(mgr2.get("number_key"), Some(SettingValue::Number(123.0)));
        assert_eq!(mgr2.get("flag_key"), Some(SettingValue::Flag(true)));

        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_settings_atomic_write() {
        let tmp = std::env::temp_dir().join(format!("atomic_test_{}.json", std::process::id()));
        let mgr = SettingsManager::with_path(&tmp).unwrap();

        mgr.set("key1", SettingValue::Text("value1".into()))
            .unwrap();

        // Verify file exists and contains the data
        assert!(tmp.exists());
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert!(content.contains("key1"));

        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_settings_load_or_default() {
        let tmp = std::env::temp_dir().join(format!("default_test_{}.json", std::process::id()));

        // When file doesn't exist, should return defaults (empty HashMap)
        let mgr = SettingsManager::with_path(&tmp).unwrap();
        assert!(mgr.get("nonexistent").is_none());

        // Set a value and verify persistence
        mgr.set("persisted_key", SettingValue::Text("initial".into()))
            .unwrap();
        assert_eq!(
            mgr.get("persisted_key"),
            Some(SettingValue::Text("initial".into()))
        );

        // Create a new manager pointing to same file to verify load
        let mgr2 = SettingsManager::with_path(&tmp).unwrap();
        assert_eq!(
            mgr2.get("persisted_key"),
            Some(SettingValue::Text("initial".into()))
        );

        // Overwrite and verify
        mgr2.set("persisted_key", SettingValue::Text("updated".into()))
            .unwrap();
        assert_eq!(
            mgr2.get("persisted_key"),
            Some(SettingValue::Text("updated".into()))
        );

        // Verify type preservation across save/load cycles
        mgr2.set("num_key", SettingValue::Number(99.0)).unwrap();
        mgr2.set("flag_key", SettingValue::Flag(true)).unwrap();

        let mgr3 = SettingsManager::with_path(&tmp).unwrap();
        assert_eq!(mgr3.get("num_key"), Some(SettingValue::Number(99.0)));
        assert_eq!(mgr3.get("flag_key"), Some(SettingValue::Flag(true)));

        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_settings_corrupted_file_loads_defaults() {
        // Write an invalid JSON file and verify load_or_default falls back to empty defaults
        let tmp = std::env::temp_dir().join(format!("corrupted_test_{}.json", std::process::id()));
        std::fs::write(&tmp, "not valid json at all!!!").unwrap();
        let mgr = SettingsManager::with_path(&tmp).unwrap();
        // Corrupted file should load as defaults, so values are empty
        assert!(mgr.get("any_key").is_none());
        // After setting a value, it should persist normally
        mgr.set("recovery", SettingValue::Text("ok".into()))
            .unwrap();
        assert_eq!(mgr.get("recovery"), Some(SettingValue::Text("ok".into())));
        // Verify clean JSON was written after recovery
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert!(content.contains("ok"));
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_settings_path_with_deep_subdir() {
        // Verify that a settings path deep in a non-existent directory tree
        // creates all intermediate directories successfully.
        let tmp = std::env::temp_dir().join(format!("deep_test_{}", std::process::id()));
        let deep = tmp.join("a").join("b").join("c").join("settings.json");
        let mgr = SettingsManager::with_path(&deep).unwrap();
        mgr.set("deep_key", SettingValue::Text("deep_value".into()))
            .unwrap();
        assert!(deep.exists(), "Deep settings path should be created");

        // Load a new manager pointing to the same deep file
        let mgr2 = SettingsManager::with_path(&deep).unwrap();
        assert_eq!(
            mgr2.get("deep_key"),
            Some(SettingValue::Text("deep_value".into()))
        );

        // Clean up
        std::fs::remove_dir_all(&tmp).ok();
    }
}
