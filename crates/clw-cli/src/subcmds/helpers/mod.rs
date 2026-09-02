//! Helpers shared by lifecycle subcommands.

use clw_types::ClwError;
use serde_json::json;

/// True if the server-regression branch was returned for a LIVE capability.
///
/// The client maps HTTP 405/501 responses to a `Transport` error carrying this
/// token. All lifecycle commands must recognize that exact branch so they do
/// not report a failed capability as an empty or successful result.
pub(crate) fn is_capability_unavailable(err: &ClwError) -> bool {
    matches!(err, ClwError::Transport(message) if message.contains("405/501"))
}

/// Render the generic LIVE-capability server-regression case.
///
/// `ls`, `prune`, and `erase` intentionally share this renderer: their frozen
/// JSON shape, message, and non-zero exit behavior are byte-identical. `rm`
/// does not use it because its JSON contract must retain `deleted: false`.
pub(crate) fn capability_unavailable(
    json_output: bool,
    capability: &str,
    dependency: &str,
) -> Result<(), ClwError> {
    let message = format_capability_unavailable(capability, dependency);
    if json_output {
        println!("{}", build_capability_unavailable_json(&message));
    }
    Err(ClwError::Transport(message))
}

/// Build the frozen generic structured failure note.
pub(crate) fn build_capability_unavailable_json(message: &str) -> serde_json::Value {
    json!({ "error": "capability-unavailable", "note": message })
}

fn format_capability_unavailable(capability: &str, dependency: &str) -> String {
    format!(
        "{capability} failed: corelink-server capability {dependency} is LIVE but returned 405/501 — likely a server-side regression; retry or escalate (see docs/RELAY-server-D1-D7-D8.md)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_unavailable_matches_only_the_frozen_405_501_branch() {
        assert!(is_capability_unavailable(&ClwError::Transport(
            "server returned 405/501 for a LIVE capability".to_string(),
        )));
        assert!(!is_capability_unavailable(&ClwError::Transport(
            "server returned 500".to_string(),
        )));
        assert!(!is_capability_unavailable(&ClwError::Auth));
    }

    #[test]
    fn generic_renderer_preserves_message_json_and_exit_contract() {
        let error = capability_unavailable(false, "ref enumeration", "D-7")
            .expect_err("a capability regression must fail");
        let message = "ref enumeration failed: corelink-server capability D-7 is LIVE but returned 405/501 — likely a server-side regression; retry or escalate (see docs/RELAY-server-D1-D7-D8.md)";
        assert!(matches!(error, ClwError::Transport(ref actual) if actual == message));
        assert_eq!(
            build_capability_unavailable_json(message).to_string(),
            format!(r#"{{"error":"capability-unavailable","note":"{message}"}}"#)
        );
    }
}
