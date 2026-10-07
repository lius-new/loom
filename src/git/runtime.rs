use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

use super::command::hide_console_window;
use super::error::{GitError, GitErrorKind, GitResult};
use super::types::{GitCapabilities, GitRuntimeSource, GitVersion};

#[derive(Clone, Debug)]
pub struct GitRuntime {
    pub executable: PathBuf,
    pub root: PathBuf,
    pub source: GitRuntimeSource,
    pub version: GitVersion,
    pub capabilities: GitCapabilities,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RuntimeManifest {
    pub version: String,
    pub platform: String,
    pub architecture: String,
    pub source: String,
    pub files: Vec<RuntimeManifestFile>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RuntimeManifestFile {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Clone, Debug, Default)]
pub struct GitRuntimeManager {
    custom_path: Option<PathBuf>,
    prefer_system: bool,
}

impl GitRuntimeManager {
    pub fn custom(path: PathBuf) -> Self {
        Self {
            custom_path: Some(path),
            prefer_system: false,
        }
    }

    pub fn prefer_system(mut self, prefer_system: bool) -> Self {
        self.prefer_system = prefer_system;
        self
    }

    pub fn resolve(&self) -> GitResult<GitRuntime> {
        if let Some(path) = &self.custom_path {
            return validate(path.clone(), GitRuntimeSource::Custom);
        }

        let managed = managed_candidates();
        let mut failures = Vec::new();
        if self.prefer_system {
            for (path, source) in system_candidates() {
                match validate(path.clone(), source) {
                    Ok(runtime) => return Ok(runtime),
                    Err(error) => failures.push(format!("{}: {error}", path.display())),
                }
            }
        }
        for (path, _) in managed {
            match validate_managed(path.clone()) {
                Ok(runtime) => return Ok(runtime),
                Err(error) => failures.push(format!("{}: {error}", path.display())),
            }
        }
        for (path, source) in system_candidates() {
            match validate(path.clone(), source) {
                Ok(runtime) => return Ok(runtime),
                Err(error) => failures.push(format!("{}: {error}", path.display())),
            }
        }
        Err(GitError::new(
            GitErrorKind::RuntimeUnavailable,
            "No usable Git runtime was found.",
        )
        .with_detail(failures.join("\n")))
    }
}

fn managed_candidates() -> Vec<(PathBuf, GitRuntimeSource)> {
    let mut roots = Vec::new();
    if let Ok(executable) = env::current_exe()
        && let Some(root) = executable.parent()
    {
        roots.push(root.to_path_buf());
    }
    // Only development builds look in the working directory: a release build
    // started from a terminal inside an untrusted repository must never run a
    // `runtime/git` that the repository provides.
    if cfg!(debug_assertions)
        && let Ok(root) = env::current_dir()
    {
        roots.push(root);
    }
    roots
        .into_iter()
        .flat_map(|root| {
            [
                root.join("runtime")
                    .join("git")
                    .join("bin")
                    .join(git_executable()),
                root.join("runtime")
                    .join("git")
                    .join("cmd")
                    .join(git_executable()),
                root.join("runtime").join("git").join(git_executable()),
            ]
        })
        .filter(|path| path.is_file())
        .map(|path| (path, GitRuntimeSource::Managed))
        .collect()
}

fn system_candidates() -> Vec<(PathBuf, GitRuntimeSource)> {
    // Passing a bare program name intentionally delegates PATH resolution to
    // the OS.  The child still runs directly and never passes through a shell.
    vec![(PathBuf::from(git_executable()), GitRuntimeSource::System)]
}

fn git_executable() -> &'static str {
    if cfg!(windows) { "git.exe" } else { "git" }
}

fn validate(executable: PathBuf, source: GitRuntimeSource) -> GitResult<GitRuntime> {
    let mut command = Command::new(&executable);
    command.arg("--version").stdin(Stdio::null());
    hide_console_window(&mut command);
    let output = command.output().map_err(|error| {
        GitError::new(
            GitErrorKind::RuntimeUnavailable,
            format!("Could not start Git: {error}"),
        )
    })?;
    if !output.status.success() {
        return Err(GitError::from_stderr(&output.stderr, output.status.code()));
    }
    let version = parse_version(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| {
        GitError::new(
            GitErrorKind::InvalidOutput,
            "Git returned an invalid version.",
        )
    })?;
    let root = runtime_root(&executable);
    if source == GitRuntimeSource::Managed {
        validate_manifest(&root)?;
    }
    Ok(GitRuntime {
        executable,
        root,
        source,
        version,
        capabilities: capabilities(version),
    })
}

fn validate_managed(executable: PathBuf) -> GitResult<GitRuntime> {
    let root = runtime_root(&executable);
    let manifest = validate_manifest_quick(&root, &executable)?;
    let version = parse_version(&manifest.version).ok_or_else(|| {
        GitError::new(
            GitErrorKind::InvalidOutput,
            "Managed Git manifest has an invalid version.",
        )
    })?;
    Ok(GitRuntime {
        executable,
        root,
        source: GitRuntimeSource::Managed,
        version,
        capabilities: capabilities(version),
    })
}

fn validate_manifest_quick(root: &Path, executable: &Path) -> GitResult<RuntimeManifest> {
    let path = root.join("MANIFEST.json");
    let contents = std::fs::read(&path).map_err(|error| {
        GitError::new(
            GitErrorKind::RuntimeUnavailable,
            format!("Managed Git manifest is missing: {error}"),
        )
    })?;
    let manifest: RuntimeManifest = serde_json::from_slice(&contents).map_err(|error| {
        GitError::new(
            GitErrorKind::InvalidOutput,
            "Managed Git manifest is invalid.",
        )
        .with_detail(error.to_string())
    })?;
    let relative = executable.strip_prefix(root).unwrap_or(executable);
    let file = manifest
        .files
        .iter()
        .find(|file| file.path == relative)
        .ok_or_else(|| {
            GitError::new(
                GitErrorKind::RuntimeUnavailable,
                "Managed Git executable is not covered by its manifest.",
            )
        })?;
    verify_manifest_file(root, file)?;
    Ok(manifest)
}

pub fn validate_manifest(root: &Path) -> GitResult<RuntimeManifest> {
    let path = root.join("MANIFEST.json");
    let contents = std::fs::read(&path).map_err(|error| {
        GitError::new(
            GitErrorKind::RuntimeUnavailable,
            format!("Managed Git manifest is missing: {error}"),
        )
    })?;
    let manifest: RuntimeManifest = serde_json::from_slice(&contents).map_err(|error| {
        GitError::new(
            GitErrorKind::InvalidOutput,
            "Managed Git manifest is invalid.",
        )
        .with_detail(error.to_string())
    })?;
    for file in &manifest.files {
        if file.path.is_absolute()
            || file
                .path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(GitError::new(
                GitErrorKind::RuntimeUnavailable,
                "Managed Git manifest contains an unsafe path.",
            ));
        }
        verify_manifest_file(root, file)?;
    }
    Ok(manifest)
}

fn verify_manifest_file(root: &Path, file: &RuntimeManifestFile) -> GitResult<()> {
    let bytes = std::fs::read(root.join(&file.path)).map_err(|error| {
        GitError::new(
            GitErrorKind::RuntimeUnavailable,
            format!(
                "Managed Git file is missing: {} ({error})",
                file.path.display()
            ),
        )
    })?;
    let digest = ring::digest::digest(&ring::digest::SHA256, &bytes);
    let actual = digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if !actual.eq_ignore_ascii_case(&file.sha256) {
        return Err(GitError::new(
            GitErrorKind::RuntimeUnavailable,
            format!(
                "Managed Git file failed verification: {}",
                file.path.display()
            ),
        ));
    }
    Ok(())
}

fn runtime_root(executable: &Path) -> PathBuf {
    executable
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf()
}

fn parse_version(output: &str) -> Option<GitVersion> {
    let token = output.split_whitespace().find(|token| {
        token.as_bytes().first().is_some_and(u8::is_ascii_digit) && token.contains('.')
    })?;
    let mut parts = token.split('.');
    Some(GitVersion {
        major: numeric_prefix(parts.next()?)?,
        minor: numeric_prefix(parts.next()?)?,
        patch: parts.next().and_then(numeric_prefix).unwrap_or(0),
    })
}

fn numeric_prefix(value: &str) -> Option<u32> {
    let digits = value
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn capabilities(version: GitVersion) -> GitCapabilities {
    let at_least = |major, minor| (version.major, version.minor) >= (major, minor);
    GitCapabilities {
        porcelain_v2: at_least(2, 11),
        switch: at_least(2, 23),
        restore: at_least(2, 23),
        pathspec_from_file: at_least(2, 11),
        force_with_lease: at_least(1, 8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_platform_version_suffixes() {
        assert_eq!(
            parse_version("git version 2.51.0.windows.1"),
            Some(GitVersion {
                major: 2,
                minor: 51,
                patch: 0
            })
        );
        assert_eq!(
            parse_version("git version 2.39.5 (Apple Git-154)"),
            Some(GitVersion {
                major: 2,
                minor: 39,
                patch: 5
            })
        );
    }
}
