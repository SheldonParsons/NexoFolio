//! Curate: turns what traffic proves into words and a place in the tree.
//!
//! Two passes, both pure functions of observe's facts plus one model call:
//!
//! - [`describe`] writes a name and a purpose for one endpoint
//! - [`organize`] groups a project's endpoints into folders
//!
//! Neither touches storage. Both return `Command`s for knowledge to apply
//! inside a round, so a pass is undoable by the same path a manual edit is.
//! That is also why the model sits behind [`Completions`]: tests decide what it
//! says, and no test needs a network.

mod describe;
mod organize;
mod prompt;

pub use describe::Describe;
pub use nexofolio_curate_contracts::{CompletionError, Completions};
pub use organize::Organize;

/// Pulls the first JSON value out of a reply.
///
/// Models wrap JSON in prose or a ```json fence however they like, and a pass
/// that rejects the reply over a stray backtick throws away a usable answer.
pub(crate) fn json_slice(reply: &str) -> Option<&str> {
    let start = reply.find(['{', '['])?;
    let opener = reply.as_bytes()[start];
    let closer = if opener == b'{' { b'}' } else { b']' };
    let end = reply.bytes().rposition(|b| b == closer)?;
    if end <= start {
        return None;
    }
    Some(&reply[start..=end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_survives_a_fenced_reply() {
        let fenced = "Here you go:\n```json\n{\"name\": \"下单\"}\n```\nHope that helps.";
        assert_eq!(json_slice(fenced), Some("{\"name\": \"下单\"}"));
    }

    #[test]
    fn json_survives_a_bare_reply() {
        assert_eq!(json_slice("{\"a\":1}"), Some("{\"a\":1}"));
    }

    #[test]
    fn arrays_are_found_too() {
        assert_eq!(json_slice("prose [1,2] tail"), Some("[1,2]"));
    }

    #[test]
    fn prose_alone_yields_nothing() {
        assert_eq!(json_slice("I cannot help with that."), None);
    }
}
