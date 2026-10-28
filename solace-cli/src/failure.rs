//! A command's failure and result type, shared by every command.

/// A command's failure: its message and exit code (1 diagnostics, 2 usage).
#[derive(Debug)]
pub(crate) struct Failure(pub(crate) String, pub(crate) i32);

pub(crate) type Out = Result<String, Failure>;

pub(crate) fn fail<T>(msg: impl Into<String>) -> Result<T, Failure> {
    Err(Failure(msg.into(), 1))
}

pub(crate) fn usage<T>(msg: impl Into<String>) -> Result<T, Failure> {
    Err(Failure(format!("{}\n\nRun `solace help` for usage.", msg.into()), 2))
}

impl From<rusqlite::Error> for Failure {
    fn from(e: rusqlite::Error) -> Self {
        Failure(format!("store error: {e}"), 1)
    }
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Failure(format!("io error: {e}"), 1)
    }
}
