use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::error::ConfigError;

pub const TEST_DATA_DIR_ENV: &str = "STICKIES_TEST_DATA_DIR";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPaths {
    pub root: PathBuf,
    pub notes: PathBuf,
    pub index: PathBuf,
}

impl DataPaths {
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let notes = root.join("notes");
        let index = root.join("index.db");

        Self { root, notes, index }
    }

    pub fn resolve() -> Result<Self, ConfigError> {
        Self::resolve_with(|key| env::var_os(key))
    }

    fn resolve_with(env: impl Fn(&str) -> Option<OsString>) -> Result<Self, ConfigError> {
        if let Some(root) = env(TEST_DATA_DIR_ENV).and_then(absolute_path) {
            return Ok(Self::with_root(root));
        }

        if let Some(data_home) = env("XDG_DATA_HOME").and_then(absolute_path) {
            return Ok(Self::with_root(data_home.join("stickies")));
        }

        let home = env("HOME")
            .and_then(absolute_path)
            .ok_or(ConfigError::MissingDataHome)?;

        Ok(Self::with_root(home.join(".local/share/stickies")))
    }

    pub fn ensure_dirs(&self) -> Result<(), ConfigError> {
        ensure_private_dir(&self.root)?;
        ensure_private_dir(&self.notes)?;
        Ok(())
    }
}

fn absolute_path(value: OsString) -> Option<PathBuf> {
    if value.is_empty() {
        return None;
    }

    let path = PathBuf::from(value);
    path.is_absolute().then_some(path)
}

fn ensure_private_dir(path: &Path) -> Result<(), ConfigError> {
    if let Err(source) = fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
    {
        return Err(ConfigError::CreateDirectory {
            path: path.to_path_buf(),
            source,
        });
    }

    if let Err(source) = fs::set_permissions(path, fs::Permissions::from_mode(0o700)) {
        return Err(ConfigError::SetPermissions {
            path: path.to_path_buf(),
            source,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::tempdir;

    use super::*;

    fn env_map(values: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let values: HashMap<String, OsString> = values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), OsString::from(value)))
            .collect();

        move |key| values.get(key).cloned()
    }

    #[test]
    fn builds_paths_from_root() {
        let paths = DataPaths::with_root("/tmp/stickies-test");

        assert_eq!(paths.root, PathBuf::from("/tmp/stickies-test"));
        assert_eq!(paths.notes, PathBuf::from("/tmp/stickies-test/notes"));
        assert_eq!(paths.index, PathBuf::from("/tmp/stickies-test/index.db"));
    }

    #[test]
    fn resolves_xdg_data_home() {
        let paths = DataPaths::resolve_with(env_map(&[
            ("XDG_DATA_HOME", "/tmp/xdg-data"),
            ("HOME", "/tmp/home"),
        ]))
        .expect("paths resolve");

        assert_eq!(paths.root, PathBuf::from("/tmp/xdg-data/stickies"));
    }

    #[test]
    fn falls_back_to_home_data_directory() {
        let paths =
            DataPaths::resolve_with(env_map(&[("HOME", "/tmp/home")])).expect("paths resolve");

        assert_eq!(paths.root, PathBuf::from("/tmp/home/.local/share/stickies"));
    }

    #[test]
    fn test_override_takes_precedence() {
        let paths = DataPaths::resolve_with(env_map(&[
            (TEST_DATA_DIR_ENV, "/tmp/test-root"),
            ("XDG_DATA_HOME", "/tmp/xdg-data"),
            ("HOME", "/tmp/home"),
        ]))
        .expect("paths resolve");

        assert_eq!(paths.root, PathBuf::from("/tmp/test-root"));
    }

    #[test]
    fn ignores_relative_xdg_data_home() {
        let paths = DataPaths::resolve_with(env_map(&[
            ("XDG_DATA_HOME", "relative/path"),
            ("HOME", "/tmp/home"),
        ]))
        .expect("paths resolve");

        assert_eq!(paths.root, PathBuf::from("/tmp/home/.local/share/stickies"));
    }

    #[test]
    fn requires_a_data_home() {
        assert!(matches!(
            DataPaths::resolve_with(env_map(&[])),
            Err(ConfigError::MissingDataHome)
        ));
    }

    #[test]
    fn creates_private_directories() {
        let temp = tempdir().expect("temp dir");
        let paths = DataPaths::with_root(temp.path().join("stickies"));

        paths.ensure_dirs().expect("directories created");
        paths.ensure_dirs().expect("directories are idempotent");

        for path in [&paths.root, &paths.notes] {
            let mode = fs::metadata(path)
                .expect("directory metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o700, "unexpected permissions for {path:?}");
        }
    }
}
