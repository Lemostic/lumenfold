//! Error type crossing the IPC boundary.
//!
//! Every failure that reaches the frontend is serialized as a tagged object so
//! the UI can branch on `kind` instead of matching on prose:
//!
//! ```json
//! { "kind": "no_library", "message": "..." }
//! ```

use std::error::Error;
use std::fmt;

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

/// Stable machine-readable discriminator for [`LumenError`] variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The requested operation needs a library index that has not been built yet.
    NotIndexed,
    /// The operation is not implemented by this build (e.g. an unknown query mode).
    Unsupported,
    /// A filesystem or network operation failed.
    Io,
    /// No library folder is open, so there is nothing to read or write.
    NoLibrary,
}

impl ErrorKind {
    /// Wire representation used as the `kind` field.
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorKind::NotIndexed => "not_indexed",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::Io => "io",
            ErrorKind::NoLibrary => "no_library",
        }
    }
}

/// Error returned by every `#[tauri::command]` that can fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LumenError {
    /// No library index exists yet. Callers should surface the "build your index"
    /// empty state rather than treating this as a bug.
    NotIndexed(String),
    /// The request is well formed but this build cannot serve it.
    Unsupported(String),
    /// An underlying I/O operation failed.
    Io(String),
    /// No library folder has been opened yet.
    NoLibrary(String),
}

impl LumenError {
    /// Convenience constructor for [`LumenError::NotIndexed`].
    pub fn not_indexed(message: impl Into<String>) -> Self {
        LumenError::NotIndexed(message.into())
    }

    /// Convenience constructor for [`LumenError::Unsupported`].
    pub fn unsupported(message: impl Into<String>) -> Self {
        LumenError::Unsupported(message.into())
    }

    /// Convenience constructor for [`LumenError::Io`].
    pub fn io(message: impl Into<String>) -> Self {
        LumenError::Io(message.into())
    }

    /// Convenience constructor for [`LumenError::NoLibrary`].
    pub fn no_library(message: impl Into<String>) -> Self {
        LumenError::NoLibrary(message.into())
    }

    /// Discriminator the frontend branches on.
    pub fn kind(&self) -> ErrorKind {
        match self {
            LumenError::NotIndexed(_) => ErrorKind::NotIndexed,
            LumenError::Unsupported(_) => ErrorKind::Unsupported,
            LumenError::Io(_) => ErrorKind::Io,
            LumenError::NoLibrary(_) => ErrorKind::NoLibrary,
        }
    }

    /// Human-readable detail. Never the only thing a caller should rely on.
    pub fn message(&self) -> &str {
        match self {
            LumenError::NotIndexed(message)
            | LumenError::Unsupported(message)
            | LumenError::Io(message)
            | LumenError::NoLibrary(message) => message,
        }
    }
}

impl fmt::Display for LumenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind().as_str(), self.message())
    }
}

impl Error for LumenError {}

impl From<std::io::Error> for LumenError {
    fn from(error: std::io::Error) -> Self {
        LumenError::Io(error.to_string())
    }
}

impl Serialize for LumenError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("LumenError", 2)?;
        state.serialize_field("kind", self.kind().as_str())?;
        state.serialize_field("message", self.message())?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_tagged_object() {
        let error = LumenError::not_indexed("library index missing");
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({
                "kind": "not_indexed",
                "message": "library index missing",
            })
        );
    }

    #[test]
    fn display_includes_kind_and_message() {
        assert_eq!(
            LumenError::io("disk gone").to_string(),
            "io: disk gone"
        );
    }

    #[test]
    fn no_library_serializes_with_its_own_kind() {
        let error = LumenError::no_library("no folder open");
        assert_eq!(error.kind(), ErrorKind::NoLibrary);
        assert_eq!(ErrorKind::NoLibrary.as_str(), "no_library");
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({
                "kind": "no_library",
                "message": "no folder open",
            })
        );
        assert_eq!(error.to_string(), "no_library: no folder open");
    }
}