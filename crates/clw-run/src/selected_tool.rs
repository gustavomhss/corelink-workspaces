//! Unix selection shared by the memo identity and the actual launch.
//! No discovery subprocess is run. This is not an immutable-file execution lease.

use super::{digest_tool_path, ClwError, Result, ToolDigestCache, ToolFingerprint};
use std::{
    ffi::{OsStr, OsString},
    io,
    os::unix::{ffi::OsStrExt, process::CommandExt},
    path::{Path, PathBuf},
};

pub(super) struct SelectedTool {
    absolute: PathBuf,
    launch: PathBuf,
    fingerprint: ToolFingerprint,
    cwd_only_path: Option<OsString>,
    pub(super) identity: String,
}

/// Capture only the PATH form whose bare-name launch is intentionally retained.
/// Keep this decision independent of ambient process state for direct coverage.
fn captured_cwd_only_path(cmd0: &str, paths: Option<&OsStr>) -> Option<OsString> {
    paths
        .filter(|paths| {
            !cmd0.contains('/')
                && std::env::split_paths(paths).all(|dir| dir.as_os_str().is_empty())
        })
        .map(OsStr::to_os_string)
}

/// Pure Linux launch-policy predicate, also exercised on non-Linux test hosts.
/// Testing it does not claim to execute the Linux launcher on those hosts.
#[cfg(any(target_os = "linux", test))]
fn mixed_empty_path_requires_explicit_tool(empty_entry: bool, cwd_only: bool) -> bool {
    empty_entry && !cwd_only
}

fn executable(path: &Path) -> io::Result<()> {
    if !std::fs::metadata(path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "tool is not a regular executable",
        ));
    }
    // Effective permission checks include ACLs; testing mode & 0o111 alone does not.
    rustix::fs::accessat(
        rustix::fs::CWD,
        path,
        rustix::fs::Access::EXEC_OK,
        rustix::fs::AtFlags::EACCESS,
    )
    .map_err(io::Error::from)
}

fn select(cwd: &Path, cmd0: &str, paths: Option<&OsStr>) -> io::Result<(PathBuf, PathBuf)> {
    if cmd0.contains('/') {
        let launch = PathBuf::from(cmd0);
        let absolute = cwd.join(&launch);
        executable(&absolute)?;
        return Ok((absolute, launch));
    }
    let paths = paths.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "PATH is not set; use an explicit executable path",
        )
    })?;
    let cwd_only = std::env::split_paths(paths).all(|dir| dir.as_os_str().is_empty());
    let mut denied = false;
    for dir in std::env::split_paths(paths) {
        let empty_entry = dir.as_os_str().is_empty();
        // An all-empty PATH has exactly one location. Retain the bare spelling:
        // prefixing "./" changes script $0 on Linux even with native arg0 set.
        let launch = if cwd_only {
            PathBuf::from(cmd0)
        } else if empty_entry {
            PathBuf::from(".").join(cmd0)
        } else {
            dir.join(cmd0)
        };
        let absolute = cwd.join(&launch);
        match executable(&absolute) {
            Ok(()) => {
                // A mixed PATH cannot bind this bare spelling through Command
                // without altering the child's PATH or Linux script $0.
                #[cfg(target_os = "linux")]
                if mixed_empty_path_requires_explicit_tool(empty_entry, cwd_only) {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "an empty entry in mixed PATH selected the tool; use an explicit ./tool path or PATH=.",
                    ));
                }
                return Ok((absolute, launch));
            }
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => denied = true,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        if denied {
            io::ErrorKind::PermissionDenied
        } else {
            io::ErrorKind::NotFound
        },
        "no executable tool found on PATH",
    ))
}

impl SelectedTool {
    pub(super) async fn resolve(cwd: &Path, cmd0: &str, cache: &ToolDigestCache) -> Result<Self> {
        let cwd = std::env::current_dir()?.join(cwd);
        let name = cmd0.to_owned();
        let paths = std::env::var_os("PATH");
        let cwd_only_path = captured_cwd_only_path(cmd0, paths.as_deref());
        let (absolute, launch) =
            tokio::task::spawn_blocking(move || select(&cwd, &name, paths.as_deref()))
                .await
                .map_err(|e| ClwError::Transport(e.to_string()))??;
        let before = ToolFingerprint::read(&absolute).await?;
        let digest = Box::pin(digest_tool_path(&absolute, cmd0, cache)).await;
        if !digest.starts_with("digest:")
            || ToolFingerprint::read(&absolute).await.ok().as_ref() != Some(&before)
        {
            return Err(ClwError::Config("selected executable cannot be identified by stable readable content; memoization refused".into()));
        }
        // A versioned tool identity separates old incorrectly selected v4 records.
        // Encode native path bytes losslessly; never let lossy display text identify a tool.
        let identity = format!(
            "unix-executable-v1:{}",
            serde_json::to_string(&(absolute.as_os_str().as_bytes(), digest))?
        );
        Ok(Self {
            absolute,
            launch,
            fingerprint: before,
            cwd_only_path,
            identity,
        })
    }

    pub(super) fn command(&self, argv0: &str) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.launch);
        command.as_std_mut().arg0(argv0);
        if let Some(path) = &self.cwd_only_path {
            // Freeze the original value, not ".". Every entry denotes cwd, so
            // this name cannot search a different directory after lookup.
            command.env("PATH", path);
        }
        command
    }

    pub(super) async fn is_current(&self) -> bool {
        let path = self.absolute.clone();
        let permitted = tokio::task::spawn_blocking(move || executable(&path).is_ok())
            .await
            .unwrap_or(false);
        permitted
            && ToolFingerprint::read(&self.absolute).await.ok().as_ref() == Some(&self.fingerprint)
    }

    pub(super) async fn verify(&self) -> Result<()> {
        if self.is_current().await {
            Ok(())
        } else {
            Err(ClwError::Config("selected executable changed during memo lookup; invocation refused before replay or dispatch".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{captured_cwd_only_path, mixed_empty_path_requires_explicit_tool};
    use std::ffi::{OsStr, OsString};

    #[test]
    fn linux_mixed_empty_entry_policy_rejects_only_the_ambiguous_selection() {
        for (empty_entry, cwd_only, expected) in [
            (false, false, false),
            (false, true, false),
            (true, false, true),
            (true, true, false),
        ] {
            assert_eq!(
                mixed_empty_path_requires_explicit_tool(empty_entry, cwd_only),
                expected
            );
        }
    }

    #[test]
    fn cwd_only_capture_preserves_exact_value_and_rejects_other_launch_forms() {
        assert_eq!(captured_cwd_only_path("tool", None), None);
        for value in ["", ":", "::"] {
            assert_eq!(
                captured_cwd_only_path("tool", Some(OsStr::new(value))),
                Some(OsString::from(value)),
                "bare-name cwd-only PATH must be captured byte-for-byte"
            );
            for explicit in ["./tool", "bin/tool", "/bin/tool"] {
                assert_eq!(
                    captured_cwd_only_path(explicit, Some(OsStr::new(value))),
                    None,
                    "an explicit path must not acquire a bare-name PATH overlay"
                );
            }
        }
        for value in ["bin", ":bin", "bin:", "bin::other"] {
            assert_eq!(
                captured_cwd_only_path("tool", Some(OsStr::new(value))),
                None,
                "a nonempty directory is not a cwd-only PATH: {value:?}"
            );
        }
    }
}
