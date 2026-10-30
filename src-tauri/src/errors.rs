use std::fmt;

use serde::Serialize;

/// Error carrying a stable `code` the frontend translates, plus a human-readable
/// English `message` used as the detail (and as fallback for unknown codes).
#[derive(Debug)]
pub struct Coded {
    pub code: &'static str,
    pub message: String,
}

impl fmt::Display for Coded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Coded {}

pub fn coded(code: &'static str, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Coded {
        code,
        message: message.into(),
    })
}

/// What a command hands back to the webview on failure.
#[derive(Debug, Serialize)]
pub struct CommandError {
    pub code: Option<&'static str>,
    pub message: String,
}

impl From<anyhow::Error> for CommandError {
    fn from(err: anyhow::Error) -> Self {
        let code = err
            .chain()
            .find_map(|cause| cause.downcast_ref::<Coded>())
            .map(|c| c.code);
        CommandError {
            code,
            // `{:#}` prints the whole chain of causes, not only the outermost context.
            message: format!("{err:#}"),
        }
    }
}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        CommandError {
            code: None,
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn the_code_survives_added_context() {
        let err = Err::<(), _>(coded("ffmpeg_missing", "no ffmpeg"))
            .context("extracting audio")
            .unwrap_err();

        let out = CommandError::from(err);

        assert_eq!(out.code, Some("ffmpeg_missing"));
    }

    #[test]
    fn the_message_shows_the_whole_chain() {
        let err = Err::<(), _>(coded("ffmpeg_missing", "no ffmpeg"))
            .context("extracting audio")
            .unwrap_err();

        let out = CommandError::from(err);

        assert_eq!(out.message, "extracting audio: no ffmpeg");
    }

    #[test]
    fn an_uncoded_error_has_no_code() {
        let out = CommandError::from(anyhow::anyhow!("boom"));

        assert_eq!(out.code, None);
        assert_eq!(out.message, "boom");
    }
}
