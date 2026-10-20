use serde::ser::{Serialize, SerializeStruct, Serializer};

/// Every failure that can happen in the backend.
///
/// Errors cross the IPC bridge as `{ kind, message }` so the interface can react to
/// specific cases (for example, asking again for the password) and show the rest as text.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Wrong master password or key file")]
    InvalidCredentials,

    #[error("This file is not a Lados vault")]
    NotAVault,

    #[error("This vault was created by a newer version of Lados (format version {0})")]
    UnsupportedVersion(u16),

    #[error("The vault file is damaged")]
    Corrupted,

    #[error("The vault is locked")]
    Locked,

    #[error("No vault is selected")]
    NoVaultSelected,

    #[error("The {0} no longer exists")]
    NotFound(&'static str),

    #[error("{0}")]
    Invalid(String),

    #[error("Import failed: {0}")]
    Import(String),

    #[error("Export failed: {0}")]
    Export(String),

    #[error("File error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Data error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Cryptography error: {0}")]
    Crypto(String),

    #[error("System error: {0}")]
    System(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Error::Invalid(message.into())
    }

    fn kind(&self) -> &'static str {
        match self {
            Error::InvalidCredentials => "invalidCredentials",
            Error::NotAVault => "notAVault",
            Error::UnsupportedVersion(_) => "unsupportedVersion",
            Error::Corrupted => "corrupted",
            Error::Locked => "locked",
            Error::NoVaultSelected => "noVaultSelected",
            Error::NotFound(_) => "notFound",
            Error::Invalid(_) => "invalid",
            Error::Import(_) => "import",
            Error::Export(_) => "export",
            Error::Io(_) => "io",
            Error::Serialization(_) => "serialization",
            Error::Crypto(_) => "crypto",
            Error::System(_) => "system",
        }
    }
}

impl From<tauri::Error> for Error {
    fn from(error: tauri::Error) -> Self {
        Error::System(error.to_string())
    }
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut error = serializer.serialize_struct("Error", 2)?;
        error.serialize_field("kind", self.kind())?;
        error.serialize_field("message", &self.to_string())?;
        error.end()
    }
}
