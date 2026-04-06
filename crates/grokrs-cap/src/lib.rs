use std::marker::PhantomData;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

pub trait TrustLevel: sealed::Sealed {
    /// Numeric rank for the trust level, used by object-safe tool filtering.
    ///
    /// Higher values indicate greater trust:
    /// - `Untrusted` = 0
    /// - `InteractiveTrusted` = 1
    /// - `AdminTrusted` = 2
    fn trust_rank() -> u8;
}

pub struct Untrusted;
pub struct InteractiveTrusted;
pub struct AdminTrusted;

impl TrustLevel for Untrusted {
    fn trust_rank() -> u8 {
        0
    }
}
impl TrustLevel for InteractiveTrusted {
    fn trust_rank() -> u8 {
        1
    }
}
impl TrustLevel for AdminTrusted {
    fn trust_rank() -> u8 {
        2
    }
}

mod sealed {
    pub trait Sealed {}

    impl Sealed for super::Untrusted {}
    impl Sealed for super::InteractiveTrusted {}
    impl Sealed for super::AdminTrusted {}
}

#[derive(Debug, Clone)]
pub struct SessionScope<T: TrustLevel> {
    _trust: PhantomData<T>,
}

impl<T: TrustLevel> Default for SessionScope<T> {
    fn default() -> Self {
        Self {
            _trust: PhantomData,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceRoot {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePath {
    relative: PathBuf,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathError {
    #[error("workspace root must be absolute: {0}")]
    RootNotAbsolute(String),
    #[error("workspace path must be relative: {0}")]
    AbsolutePath(String),
    #[error("workspace path must not escape the workspace: {0}")]
    EscapesWorkspace(String),
    #[error("workspace path must not be empty")]
    EmptyPath,
}

impl WorkspaceRoot {
    /// # Errors
    ///
    /// Returns [`PathError::RootNotAbsolute`] if the given path is not absolute.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, PathError> {
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(PathError::RootNotAbsolute(path.display().to_string()));
        }
        Ok(Self {
            root: path.to_path_buf(),
        })
    }

    #[must_use]
    pub fn join(&self, path: &WorkspacePath) -> PathBuf {
        self.root.join(&path.relative)
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.root
    }
}

impl WorkspacePath {
    /// # Errors
    ///
    /// Returns [`PathError::EmptyPath`] if the path is empty.
    /// Returns [`PathError::AbsolutePath`] if the path is absolute.
    /// Returns [`PathError::EscapesWorkspace`] if the path contains `..`,
    /// a root directory, or a prefix component.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, PathError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(PathError::EmptyPath);
        }
        if path.is_absolute() {
            return Err(PathError::AbsolutePath(path.display().to_string()));
        }
        for component in path.components() {
            if matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            ) {
                return Err(PathError::EscapesWorkspace(path.display().to_string()));
            }
        }
        Ok(Self {
            relative: path.to_path_buf(),
        })
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.relative
    }
}

#[cfg(test)]
mod tests {
    use super::{PathError, WorkspacePath};

    #[test]
    fn rejects_absolute_paths() {
        let err = WorkspacePath::new("/tmp/outside").expect_err("absolute paths must fail");
        assert_eq!(err, PathError::AbsolutePath("/tmp/outside".into()));
    }

    #[test]
    fn rejects_escaping_paths() {
        let err = WorkspacePath::new("../outside").expect_err("escaping paths must fail");
        assert_eq!(err, PathError::EscapesWorkspace("../outside".into()));
    }

    #[test]
    fn accepts_normal_relative_paths() {
        let path = WorkspacePath::new("docs/specs/00_SPEC.md").expect("path should be valid");
        assert_eq!(path.as_path(), Path::new("docs/specs/00_SPEC.md"));
    }

    use std::path::Path;
}
