use crate::env::LaunchEnv;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::os::unix::io::FromRawFd;
use std::os::unix::io::IntoRawFd;
use std::os::unix::process::CommandExt;
use std::process::Command;

/// Executes the executable at the given path, capturing environment variables written to File Descriptor 3.
/// The executable should implement the exec.d interface, writing TOML key-value pairs to FD 3.
pub fn run_exec_d(path: &str, env: &LaunchEnv) -> Result<HashMap<String, String>, String> {
    let (reader_fd, writer_fd) =
        rustix::pipe::pipe().map_err(|e| format!("Failed to create OS pipe: {}", e))?;

    let reader: File = reader_fd.into();
    let writer: File = writer_fd.into();

    let writer_fd = writer.as_raw_fd();

    let mut cmd = Command::new(path);
    cmd.envs(env.vars());

    unsafe {
        cmd.pre_exec(move || {
            // Duplicate the write end of the pipe to File Descriptor 3 in the child process
            let borrowed_writer = std::os::unix::io::BorrowedFd::borrow_raw(writer_fd);

            // We construct an OwnedFd for FD 3, which dup2 requires as the target.
            // This is unsafe because we are claiming ownership of FD 3, which is fine
            // since we are in the child process and about to either overwrite it or fail.
            let mut target = std::os::unix::io::OwnedFd::from_raw_fd(3);

            if let Err(e) = rustix::io::dup2(borrowed_writer, &mut target) {
                // Leak target so it doesn't try to close an invalid FD on error
                let _ = target.into_raw_fd();
                return Err(e.into());
            }

            // Leak target so FD 3 stays open for the exec'd process!
            let _ = target.into_raw_fd();
            Ok(())
        });
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn exec.d binary '{}': {}", path, e))?;

    // CRITICAL: Close our copy of the writer in the parent so the reader will receive EOF
    // once the child process closes its copy (e.g. upon exiting or explicitly closing FD 3).
    drop(writer);

    let mut toml_output = String::new();
    let mut r = reader;
    r.read_to_string(&mut toml_output)
        .map_err(|e| format!("Failed to read FD 3 output from exec.d: {}", e))?;

    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for exec.d child process: {}", e))?;

    if !status.success() {
        return Err(format!(
            "exec.d binary '{}' failed with status: {}",
            path, status
        ));
    }

    if toml_output.trim().is_empty() {
        return Ok(HashMap::new());
    }

    let env_vars: HashMap<String, String> = toml::from_str(&toml_output).map_err(|e| {
        format!(
            "Failed to decode TOML output from exec.d binary '{}': {}\nOutput: '{}'",
            path, e, toml_output
        )
    })?;

    Ok(env_vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    #[test]
    fn test_exec_d_runner() {
        let dir = tempdir().unwrap();
        let script_path = dir.path().join("mock_exec_d.sh");

        // Write a simple bash script that outputs to FD 3
        let script_content = r#"#!/bin/bash
echo 'MY_NEW_VAR = "injected_value"' >&3
"#;
        fs::write(&script_path, script_content).unwrap();

        // Make the script executable
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();

        let env = LaunchEnv::new(&[], "", "");
        let res = run_exec_d(&script_path.to_string_lossy(), &env);

        assert!(res.is_ok(), "Failed to run exec.d: {:?}", res.err());
        let vars = res.unwrap();
        assert_eq!(vars.get("MY_NEW_VAR").unwrap(), "injected_value");
    }
}
