use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Represents the environment modification action type defined by the CNB specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionType {
    /// Overwrites any existing value of the variable.
    Override,
    /// Sets the variable only if it does not already exist.
    Default,
    /// Appends the new value to the end of the variable, using an optional custom delimiter.
    Append,
    /// Prepends the new value to the beginning of the variable, using an optional custom delimiter.
    Prepend,
}

/// The list of environment variables that are explicitly excluded from leaking into the final launch process environment.
pub const LAUNCH_ENV_EXCLUDELIST: &[&str] = &[
    "CNB_LAYERS_DIR",
    "CNB_APP_DIR",
    "CNB_PROCESS_TYPE",
    "CNB_PLATFORM_API",
    "CNB_DEPRECATION_MODE",
];

/// Encapsulates the execution environment variables and layer-sourcing modifications for the launch process.
pub struct LaunchEnv {
    vars: HashMap<String, String>,
    root_dir_map: HashMap<String, Vec<String>>,
}

impl LaunchEnv {
    /// Creates a new `LaunchEnv` populated from the host environment variables.
    /// Excludes variables defined in `LAUNCH_ENV_EXCLUDELIST` and sanitizes the `PATH`
    /// by stripping out the `process_dir` and `lifecycle_dir` to prevent runtime pollution.
    pub fn new(environ: &[(String, String)], process_dir: &str, lifecycle_dir: &str) -> Self {
        let mut vars = HashMap::new();

        for (k, v) in environ {
            if LAUNCH_ENV_EXCLUDELIST.contains(&k.as_str()) {
                continue;
            }
            vars.insert(k.clone(), v.clone());
        }

        // Sanitize PATH
        if let Some(path_val) = vars.get("PATH") {
            let parts = std::env::split_paths(&path_val);
            let mut stripped = Vec::new();
            for part in parts {
                if part.to_str() == Some(process_dir) || part.to_str() == Some(lifecycle_dir) {
                    continue;
                }
                stripped.push(part);
            }
            if let Ok(new_path) = std::env::join_paths(stripped) {
                vars.insert("PATH".to_string(), new_path.to_string_lossy().into_owned());
            }
        }

        let mut root_dir_map = HashMap::new();
        root_dir_map.insert("bin".to_string(), vec!["PATH".to_string()]);
        root_dir_map.insert("lib".to_string(), vec!["LD_LIBRARY_PATH".to_string()]);

        LaunchEnv { vars, root_dir_map }
    }

    /// Sets an environment variable value directly.
    pub fn set(&mut self, k: &str, v: &str) {
        self.vars.insert(k.to_string(), v.to_string());
    }

    /// Gets an environment variable value.
    pub fn get(&self, k: &str) -> Option<&String> {
        self.vars.get(k)
    }

    /// Appends a root layer path to standard PATH and LD_LIBRARY_PATH variables.
    pub fn add_root_dir(&mut self, layer_dir: &str) -> Result<(), String> {
        let abs_dir = fs::canonicalize(layer_dir)
            .map_err(|e| format!("Canonicalize layer dir '{}': {}", layer_dir, e))?;

        for (sub_dir, vars) in &self.root_dir_map {
            let child_dir = abs_dir.join(sub_dir);
            if child_dir.is_dir() {
                let child_str = child_dir.to_string_lossy().into_owned();
                for var_name in vars {
                    let current = self.vars.get(var_name).cloned().unwrap_or_default();
                    if current.is_empty() {
                        self.vars.insert(var_name.clone(), child_str.clone());
                    } else {
                        // Prepend layer path using standard PATH separator
                        let mut paths = vec![PathBuf::from(&child_str)];
                        paths.extend(std::env::split_paths(&current));
                        if let Ok(new_path) = std::env::join_paths(paths) {
                            self.vars
                                .insert(var_name.clone(), new_path.to_string_lossy().into_owned());
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Processes a directory containing environment files and applies them sequentially.
    pub fn add_env_dir(&mut self, env_dir: &str, default_action: ActionType) -> Result<(), String> {
        let path = Path::new(env_dir);
        if !path.is_dir() {
            return Ok(());
        }

        let entries =
            fs::read_dir(path).map_err(|e| format!("List env dir '{}': {}", env_dir, e))?;

        // Go reads directories in sorted order
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let ftype = entry.file_type().map_err(|e| e.to_string())?;
            if ftype.is_dir() {
                continue;
            }
            // Dereference symlinks to check if they point to directories
            if ftype.is_symlink() {
                if let Ok(metadata) = fs::metadata(entry.path()) {
                    if metadata.is_dir() {
                        continue;
                    }
                }
            }
            files.push(entry);
        }
        files.sort_by_key(|f| f.file_name());

        for file in files {
            let file_name = file.file_name().to_string_lossy().into_owned();
            let file_path = file.path();

            // Suffix parsing
            let parts: Vec<&str> = file_name.splitn(2, '.').collect();
            let name = parts[0].to_string();
            let suffix = if parts.len() > 1 { parts[1] } else { "" };

            // Delimiter files are ignored in the main action loop
            if suffix == "delim" {
                continue;
            }

            let action = match suffix {
                "override" => ActionType::Override,
                "default" => ActionType::Default,
                "append" => ActionType::Append,
                "prepend" => ActionType::Prepend,
                "" => default_action,
                _ => continue, // Ignore files with unknown suffixes
            };

            let raw_val = fs::read_to_string(&file_path)
                .map_err(|e| format!("Read env file '{}': {}", file_path.display(), e))?;

            // Spec: File contents must not contain leading/trailing whitespaces trimmed
            // (or rather, "file contents MUST NOT be evaluated by a shell or otherwise modified")
            let v = raw_val;

            let current = self.vars.get(&name).cloned().unwrap_or_default();

            match action {
                ActionType::Override => {
                    self.vars.insert(name, v);
                }
                ActionType::Default => {
                    if current.is_empty() {
                        self.vars.insert(name, v);
                    }
                }
                ActionType::Append => {
                    let d = get_delim(env_dir, &name, "");
                    let new_val = if current.is_empty() {
                        v
                    } else {
                        format!("{}{}{}", current, d, v)
                    };
                    self.vars.insert(name, new_val);
                }
                ActionType::Prepend => {
                    // Path prepending is mapped under unsuffixed env directories which resolves to PrependPath in Go.
                    // If name is a root env variable (like PATH), default delimiter is PATH separator.
                    let is_path_var = name == "PATH" || name == "LD_LIBRARY_PATH";
                    let default_delim = if is_path_var {
                        if cfg!(windows) { ";" } else { ":" }
                    } else {
                        ""
                    };

                    let d = get_delim(env_dir, &name, default_delim);
                    let new_val = if current.is_empty() {
                        v
                    } else {
                        format!("{}{}{}", v, d, current)
                    };
                    self.vars.insert(name, new_val);
                }
            }
        }
        Ok(())
    }

    /// Returns a reference to the internal environment variable map.
    pub fn vars(&self) -> &HashMap<String, String> {
        &self.vars
    }
}


fn get_delim(dir: &str, name: &str, default_delim: &str) -> String {
    let delim_path = Path::new(dir).join(format!("{}.delim", name));
    if delim_path.is_file() {
        fs::read_to_string(&delim_path).unwrap_or_else(|_| default_delim.to_string())
    } else {
        default_delim.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_new_launch_env_purging_and_sanitization() {
        let host_env = vec![
            (
                "PATH".to_string(),
                "/lifecycle:/process:/usr/bin".to_string(),
            ),
            ("CNB_APP_DIR".to_string(), "/workspace".to_string()),
            ("FOO".to_string(), "bar".to_string()),
        ];
        let env = LaunchEnv::new(&host_env, "/process", "/lifecycle");

        assert!(env.get("CNB_APP_DIR").is_none());
        assert_eq!(env.get("FOO").unwrap(), "bar");
        assert_eq!(env.get("PATH").unwrap(), "/usr/bin");
    }

    #[test]
    fn test_add_env_dir_override_and_default() {
        let dir = tempdir().unwrap();
        let dir_path = dir.path();

        // 1. Override suffix
        fs::write(dir_path.join("FOO"), "unsuffixed_val").unwrap();
        fs::write(dir_path.join("BAR.override"), "override_val").unwrap();

        let mut env = LaunchEnv::new(&[], "", "");
        env.set("FOO", "original_foo");
        env.set("BAR", "original_bar");

        env.add_env_dir(&dir_path.to_string_lossy(), ActionType::Override)
            .unwrap();

        assert_eq!(env.get("FOO").unwrap(), "unsuffixed_val");
        assert_eq!(env.get("BAR").unwrap(), "override_val");

        // 2. Default suffix
        let dir2 = tempdir().unwrap();
        let dir2_path = dir2.path();
        fs::write(dir2_path.join("FOO.default"), "default_val").unwrap();
        fs::write(dir2_path.join("BAZ.default"), "default_val").unwrap();

        env.add_env_dir(&dir2_path.to_string_lossy(), ActionType::Override)
            .unwrap();

        // FOO already exists, so default does not override it
        assert_eq!(env.get("FOO").unwrap(), "unsuffixed_val");
        // BAZ does not exist, so it gets set
        assert_eq!(env.get("BAZ").unwrap(), "default_val");
    }

    #[test]
    fn test_add_env_dir_append_and_prepend() {
        let dir = tempdir().unwrap();
        let dir_path = dir.path();

        fs::write(dir_path.join("PATH.prepend"), "/layer/bin").unwrap();
        fs::write(dir_path.join("VAR.append"), "appendage").unwrap();
        fs::write(dir_path.join("VAR.delim"), "-").unwrap();

        let mut env = LaunchEnv::new(&[], "", "");
        env.set("PATH", "/usr/bin");
        env.set("VAR", "base");

        env.add_env_dir(&dir_path.to_string_lossy(), ActionType::Override)
            .unwrap();

        // PATH uses default separator (":" on unix)
        assert_eq!(env.get("PATH").unwrap(), "/layer/bin:/usr/bin");
        // VAR uses custom delimiter "-"
        assert_eq!(env.get("VAR").unwrap(), "base-appendage");
    }
}
