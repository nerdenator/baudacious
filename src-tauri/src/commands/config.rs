//! Configuration persistence commands
//!
//! Save/load/list/delete configuration profiles as JSON files
//! in the platform-appropriate app data directory.

use crate::commands::radio::with_radio;
use crate::domain::Configuration;
use crate::state::AppState;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};

/// Get (and create if needed) the configs directory.
/// Think of this like Python's `os.makedirs(path, exist_ok=True)` — it ensures
/// the directory exists and returns the path.
fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    let dir = base.join("configs");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create configs dir: {e}"))?;
    Ok(dir)
}

/// Sanitize a configuration name to prevent path traversal.
/// Like Python's `os.path.basename()` check — rejects anything with
/// path separators, "..", or empty strings.
fn sanitize_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Configuration name cannot be empty".to_string());
    }
    if trimmed.contains("..") || trimmed.contains('/') || trimmed.contains('\\') {
        return Err("Invalid configuration name".to_string());
    }
    // Only allow alphanumeric, spaces, hyphens, underscores
    if !trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_')
    {
        return Err("Configuration name contains invalid characters".to_string());
    }
    Ok(trimmed.to_string())
}

/// Write a Configuration to a directory (path-based, testable without AppHandle).
fn write_config_to_dir(dir: &std::path::Path, config: &Configuration) -> Result<(), String> {
    let name = sanitize_name(&config.name)?;
    let path = dir.join(format!("{name}.json"));
    let json =
        serde_json::to_string_pretty(config).map_err(|e| format!("Serialization error: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to write config: {e}"))?;
    Ok(())
}

/// Write a Configuration to disk without re-checking the name (caller is responsible).
/// Extracted so set_tx_power_config can reuse it without duplicating I/O logic.
fn write_config_to_disk(app: &AppHandle, config: &Configuration) -> Result<(), String> {
    let dir = config_dir(app)?;
    write_config_to_dir(&dir, config)
}

#[tauri::command]
pub fn save_configuration(app: AppHandle, config: Configuration) -> Result<(), String> {
    write_config_to_disk(&app, &config)
}

fn load_config_from_dir(dir: &std::path::Path, name: &str) -> Result<Configuration, String> {
    let name = sanitize_name(name)?;
    let path = dir.join(format!("{name}.json"));
    let json = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read config '{name}': {e}"))?;
    serde_json::from_str(&json).map_err(|e| format!("Failed to parse config '{name}': {e}"))
}

fn list_configs_in_dir(dir: &std::path::Path) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read configs dir: {e}"))?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()?.to_str()? != "json" {
                return None;
            }
            let name = path.file_stem()?.to_string_lossy().into_owned();
            match sanitize_name(&name) {
                Ok(sanitized) if sanitized == name => Some(name),
                _ => None,
            }
        })
        .collect();
    names.sort();
    Ok(names)
}

fn delete_config_from_dir(dir: &std::path::Path, name: &str) -> Result<(), String> {
    let name = sanitize_name(name)?;
    if name == "Default" {
        return Err("Cannot delete the Default configuration".to_string());
    }
    let path = dir.join(format!("{name}.json"));
    if !path.exists() {
        return Err(format!("Configuration '{name}' not found"));
    }
    std::fs::remove_file(&path).map_err(|e| format!("Failed to delete config '{name}': {e}"))
}

/// Validate a TX power value (0–100 W inclusive).
/// Extracted so it can be tested without a Tauri app handle.
fn validate_tx_power(watts: u32) -> Result<(), String> {
    if watts > 100 {
        return Err(format!("TX power {watts} W exceeds maximum (100 W)"));
    }
    Ok(())
}

/// Update tx_power_watts in the running modem config and persist the current profile.
///
/// Updates both the in-memory `AppState.config` (used immediately by `start_tx`)
/// and the "Default" profile file on disk (so the setting survives restart).
#[tauri::command]
pub fn set_tx_power_config(
    app: AppHandle,
    watts: u32,
    state: State<AppState>,
) -> Result<(), String> {
    validate_tx_power(watts)?;

    // Update the in-memory modem config so start_tx uses the new value immediately
    {
        let mut cfg = state
            .config
            .lock()
            .map_err(|_| "config lock poisoned".to_string())?;
        cfg.tx_power_watts = watts;
    }

    // Send CAT command to radio immediately (non-fatal — radio may not be connected)
    let _ = with_radio(&state, &app, |radio| radio.set_tx_power(watts));

    // Persist: load current profile from disk, patch tx_power_watts, save back.
    // This is best-effort — if no profile file exists yet, skip silently.
    let name = "Default";
    let dir = config_dir(&app)?;
    let path = dir.join(format!("{name}.json"));
    if path.exists() {
        let mut profile = load_config_from_dir(&dir, name)?;
        profile.tx_power_watts = watts;
        write_config_to_dir(&dir, &profile)?;
    }

    Ok(())
}

#[tauri::command]
pub fn load_configuration(app: AppHandle, name: String) -> Result<Configuration, String> {
    let dir = config_dir(&app)?;
    load_config_from_dir(&dir, &name)
}

#[tauri::command]
pub fn list_configurations(app: AppHandle) -> Result<Vec<String>, String> {
    let dir = config_dir(&app)?;
    list_configs_in_dir(&dir)
}

#[tauri::command]
pub fn delete_configuration(app: AppHandle, name: String) -> Result<(), String> {
    let dir = config_dir(&app)?;
    delete_config_from_dir(&dir, &name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_name_rejects_path_traversal() {
        assert!(sanitize_name("../evil").is_err());
        assert!(sanitize_name("foo/bar").is_err());
        assert!(sanitize_name("foo\\bar").is_err());
        assert!(sanitize_name("").is_err());
        assert!(sanitize_name("  ").is_err());
    }

    #[test]
    fn sanitize_name_accepts_valid_names() {
        assert_eq!(sanitize_name("Default").unwrap(), "Default");
        assert_eq!(sanitize_name("FT-991A Home").unwrap(), "FT-991A Home");
        assert_eq!(sanitize_name("my_config_2").unwrap(), "my_config_2");
    }

    #[test]
    fn sanitize_name_rejects_special_characters() {
        assert!(sanitize_name("config<>").is_err());
        assert!(sanitize_name("config;drop").is_err());
        assert!(sanitize_name("config|pipe").is_err());
    }

    #[test]
    fn set_tx_power_config_rejects_over_100w() {
        assert!(validate_tx_power(101).is_err());
        let err = validate_tx_power(101).unwrap_err();
        assert!(err.contains("exceeds maximum"));
    }

    #[test]
    fn set_tx_power_config_accepts_0w() {
        assert!(validate_tx_power(0).is_ok());
    }

    #[test]
    fn set_tx_power_config_accepts_boundary_values() {
        assert!(validate_tx_power(25).is_ok());
        assert!(validate_tx_power(100).is_ok());
    }

    #[test]
    fn modem_config_tx_power_can_be_updated() {
        use crate::domain::ModemConfig;
        let mut cfg = ModemConfig::default();
        assert_eq!(cfg.tx_power_watts, 25); // default
        cfg.tx_power_watts = 10;
        assert_eq!(cfg.tx_power_watts, 10);
        cfg.tx_power_watts = 0;
        assert_eq!(cfg.tx_power_watts, 0);
    }

    // --- I/O path tests using temp directories ---

    fn sample_config(name: &str) -> Configuration {
        Configuration { name: name.to_string(), ..Configuration::default() }
    }

    #[test]
    fn write_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config = sample_config("Default");
        write_config_to_dir(dir.path(), &config).unwrap();
        let loaded = load_config_from_dir(dir.path(), "Default").unwrap();
        assert_eq!(loaded.name, "Default");
        assert_eq!(loaded.carrier_freq, 1000.0);
    }

    #[test]
    fn list_returns_saved_names_sorted() {
        let dir = tempfile::tempdir().unwrap();
        write_config_to_dir(dir.path(), &sample_config("Zeta")).unwrap();
        write_config_to_dir(dir.path(), &sample_config("Alpha")).unwrap();
        write_config_to_dir(dir.path(), &sample_config("Default")).unwrap();
        let names = list_configs_in_dir(dir.path()).unwrap();
        assert_eq!(names, vec!["Alpha", "Default", "Zeta"]);
    }

    #[test]
    fn list_ignores_non_json_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "ignore me").unwrap();
        write_config_to_dir(dir.path(), &sample_config("Default")).unwrap();
        let names = list_configs_in_dir(dir.path()).unwrap();
        assert_eq!(names, vec!["Default"]);
    }

    #[test]
    fn delete_removes_config() {
        let dir = tempfile::tempdir().unwrap();
        write_config_to_dir(dir.path(), &sample_config("Temp")).unwrap();
        delete_config_from_dir(dir.path(), "Temp").unwrap();
        assert!(load_config_from_dir(dir.path(), "Temp").is_err());
    }

    #[test]
    fn delete_default_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        write_config_to_dir(dir.path(), &sample_config("Default")).unwrap();
        let err = delete_config_from_dir(dir.path(), "Default").unwrap_err();
        assert!(err.contains("Cannot delete the Default"));
    }

    #[test]
    fn load_nonexistent_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_config_from_dir(dir.path(), "Ghost").unwrap_err();
        assert!(err.contains("Failed to read config"));
    }

    #[test]
    fn delete_nonexistent_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = delete_config_from_dir(dir.path(), "Ghost").unwrap_err();
        assert!(err.contains("not found"));
    }
}
