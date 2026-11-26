// A settings write that failed after bytes may have reached the device.
//
// The person's sentence has to survive the trip to the API answer (#572), while the hub still
// has to know that the link failed so it drops the values it kept. Both ride on `Io`: the
// sentence is the error's source, which the daemon can read back, and `Io` is what the hub's
// `device_failure` looks for.

use std::fmt;

use canvas_proto::traits::AdapterError;

/// A write failure whose message was written for the person, with no technical prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteFailure(pub String);

impl fmt::Display for WriteFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WriteFailure {}

/// Wraps a person-written sentence as an `Io`, so the hub still treats the write as a device
/// failure and drops what it kept. The adapter logs the technical cause separately.
pub fn write_failure(message: impl Into<String>) -> AdapterError {
    AdapterError::Io(std::io::Error::other(WriteFailure(message.into())))
}

/// The person-written sentence of a write failure, when this error carries one.
pub fn write_words(error: &AdapterError) -> Option<&str> {
    let AdapterError::Io(cause) = error else {
        return None;
    };
    cause
        .get_ref()
        .and_then(|source| source.downcast_ref::<WriteFailure>())
        .map(|failure| failure.0.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sentence_survives_the_io_wrapper() {
        let error = write_failure("The change may be only partly on the device. Press Read again.");
        assert_eq!(
            write_words(&error),
            Some("The change may be only partly on the device. Press Read again.")
        );
        assert!(matches!(error, AdapterError::Io(_)), "the hub still sees a link failure");
    }

    #[test]
    fn a_plain_io_error_has_no_person_sentence() {
        let error = AdapterError::Io(std::io::Error::other("write failed"));
        assert_eq!(write_words(&error), None);
    }
}
