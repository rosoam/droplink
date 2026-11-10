use droplink_core::config::Config;
use droplink_core::Error;
use std::path::PathBuf;

pub struct Paths {
    pub dir: PathBuf,
}

impl Paths {
    pub fn config_file(&self) -> PathBuf {
        self.dir.join("config.json")
    }

    pub fn history_file(&self) -> PathBuf {
        self.dir.join("history.json")
    }

    pub fn log_file(&self) -> PathBuf {
        self.dir.join("droplink.log")
    }
}

pub fn default_paths() -> Paths {
    let dir = directories::ProjectDirs::from("com", "droplink", "DropLink")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".droplink"));
    Paths { dir }
}

fn default_config() -> Config {
    let name = hostname::get()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|_| "DropLink".to_string());
    let receive_dir = directories::UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(|dir| dir.join("DropLink")))
        .unwrap_or_else(|| PathBuf::from("DropLink"));
    let name: String = name.chars().take(64).collect();
    Config::new(
        droplink_core::config::generate_device_id(),
        if name.trim().is_empty() {
            "DropLink".into()
        } else {
            name
        },
        receive_dir,
    )
}

pub fn load_or_create() -> (Config, Paths) {
    let paths = default_paths();
    let config = load_or_create_for(&paths);
    (config, paths)
}

pub fn load_or_create_for(paths: &Paths) -> Config {
    let _ = std::fs::create_dir_all(&paths.dir);

    let loaded = std::fs::read_to_string(paths.config_file())
        .ok()
        .and_then(|raw| serde_json::from_str::<Config>(&raw).ok())
        .filter(|config| config.validate().is_ok());

    let config = loaded.unwrap_or_else(default_config);
    if let Err(err) = save_config(paths, &config) {
        tracing::warn!(error = %err, "could not persist config");
    }
    config
}

pub fn save_config(paths: &Paths, config: &Config) -> droplink_core::Result<()> {
    std::fs::create_dir_all(&paths.dir)?;
    let raw = serde_json::to_vec_pretty(config).map_err(|e| Error::Protocol(e.to_string()))?;
    let path = paths.config_file();
    std::fs::write(&path, raw)?;
    restrict_permissions(&path);
    Ok(())
}

pub fn load_history(paths: &Paths) -> Vec<crate::state::HistoryEntry> {
    std::fs::read_to_string(paths.history_file())
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<crate::state::HistoryEntry>>(&raw).ok())
        .unwrap_or_default()
}

pub fn save_history(paths: &Paths, history: &[crate::state::HistoryEntry]) {
    let Ok(raw) = serde_json::to_vec_pretty(history) else {
        return;
    };
    if let Err(err) = std::fs::write(paths.history_file(), raw) {
        tracing::warn!(error = %err, "could not persist history");
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &std::path::Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::HistoryEntry;
    use droplink_core::protocol::{Direction, TransportMode};

    #[test]
    fn config_and_history_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            dir: dir.path().to_path_buf(),
        };
        let mut config = load_or_create_for(&paths);
        config.device_name = "Persisté".into();
        save_config(&paths, &config).unwrap();
        let raw = std::fs::read_to_string(paths.config_file()).unwrap();
        let parsed: Config = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.device_name, "Persisté");

        let history = vec![HistoryEntry {
            file_name: "a.bin".into(),
            direction: Direction::Send,
            ok: true,
            path: Some("/tmp/a.bin".into()),
            transport: TransportMode::Direct,
            bytes: 42,
        }];
        save_history(&paths, &history);
        let loaded = load_history(&paths);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].file_name, "a.bin");
    }

    #[test]
    fn invalid_config_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            dir: dir.path().to_path_buf(),
        };
        std::fs::write(paths.config_file(), b"{not json").unwrap();
        let config = load_or_create_for(&paths);
        assert!(config.validate().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn config_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            dir: dir.path().to_path_buf(),
        };
        let config = load_or_create_for(&paths);
        save_config(&paths, &config).unwrap();
        let mode = std::fs::metadata(paths.config_file())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
