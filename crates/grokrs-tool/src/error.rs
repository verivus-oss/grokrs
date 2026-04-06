use std::fmt;

use grokrs_cap::PathError;

/// Unified error type for tool classification and execution failures.
#[derive(Debug)]
pub enum ToolError {
    /// A workspace-relative path failed validation (absolute, `..` traversal, empty).
    PathValidation(PathError),
    /// An I/O operation failed (file not found, permission denied at OS level, etc.).
    Io(std::io::Error),
    /// A tool execution exceeded its configured timeout.
    Timeout {
        /// Human-readable description of what timed out.
        operation: String,
        /// The timeout duration that was exceeded.
        duration: std::time::Duration,
    },
    /// The operation was denied at the capability level (distinct from policy denial).
    PermissionDenied {
        /// What was denied.
        operation: String,
        /// Why it was denied.
        reason: String,
    },
    /// Catch-all for tool-specific errors that don't fit the above categories.
    Other(String),
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::PathValidation(e) => write!(f, "path validation failed: {e}"),
            ToolError::Io(e) => write!(f, "I/O error: {e}"),
            ToolError::Timeout {
                operation,
                duration,
            } => write!(
                f,
                "timeout after {:.1}s: {operation}",
                duration.as_secs_f64()
            ),
            ToolError::PermissionDenied { operation, reason } => {
                write!(f, "permission denied for {operation}: {reason}")
            }
            ToolError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ToolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ToolError::PathValidation(e) => Some(e),
            ToolError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<PathError> for ToolError {
    fn from(e: PathError) -> Self {
        ToolError::PathValidation(e)
    }
}

impl From<std::io::Error> for ToolError {
    fn from(e: std::io::Error) -> Self {
        ToolError::Io(e)
    }
}

impl PartialEq for ToolError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ToolError::PathValidation(a), ToolError::PathValidation(b)) => a == b,
            (ToolError::Io(a), ToolError::Io(b)) => a.kind() == b.kind(),
            (
                ToolError::Timeout {
                    operation: a_op,
                    duration: a_dur,
                },
                ToolError::Timeout {
                    operation: b_op,
                    duration: b_dur,
                },
            ) => a_op == b_op && a_dur == b_dur,
            (
                ToolError::PermissionDenied {
                    operation: a_op,
                    reason: a_r,
                },
                ToolError::PermissionDenied {
                    operation: b_op,
                    reason: b_r,
                },
            ) => a_op == b_op && a_r == b_r,
            (ToolError::Other(a), ToolError::Other(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for ToolError {}
