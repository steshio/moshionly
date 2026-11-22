//! Turning a conversation into the single prompt an agent CLI takes.

use std::path::{Path, PathBuf};

use crate::chat::{ChatMessage, ChatRequest, Role};
use crate::error::{AiError, ErrorKind};
use crate::paths::default_working_dir;

/// The text to hand to an agent CLI for this request.
///
/// When a session is being resumed the agent already holds the conversation,
/// so only the new user message is sent. Without a session (a first message,
/// or a conversation that began with another provider) the earlier turns are
/// written out ahead of the new message so the agent has the context.
pub(crate) fn cli_prompt(request: &ChatRequest) -> Result<String, AiError> {
    let (last_index, last) = last_user_message(request)?;
    let earlier = &request.messages[..last_index];
    if request.session_id.is_some() || earlier.is_empty() {
        return Ok(last.text.clone());
    }

    let mut prompt = String::from(
        "This conversation is already under way. The earlier messages follow, oldest first.\n\n",
    );
    for message in earlier {
        let speaker = match message.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        prompt.push_str(&format!("[{speaker}]\n{}\n\n", message.text.trim()));
    }
    prompt.push_str("Now answer the user's newest message:\n\n");
    prompt.push_str(&last.text);
    Ok(prompt)
}

/// The message being answered, and its position in the conversation.
pub(crate) fn last_user_message(request: &ChatRequest) -> Result<(usize, &ChatMessage), AiError> {
    request
        .messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, message)| message.role == Role::User)
        .ok_or_else(|| {
            AiError::from_kind(
                ErrorKind::InvalidRequest,
                "the conversation has no user message to answer",
            )
        })
}

/// The directory to run the agent in, created if it does not exist yet.
pub(crate) fn prepare_working_dir(request: &ChatRequest) -> Result<PathBuf, AiError> {
    let dir = request
        .working_dir
        .clone()
        .unwrap_or_else(default_working_dir);
    create_dir(&dir)?;
    Ok(dir)
}

fn create_dir(dir: &Path) -> Result<(), AiError> {
    std::fs::create_dir_all(dir).map_err(|error| {
        AiError::other(format!(
            "could not create the agent's working directory {}: {error}",
            dir.display()
        ))
    })
}

/// Checks that an MCP server name is safe to pass on a command line and to
/// build tool names from.
pub(crate) fn validate_mcp_name(name: &str) -> Result<(), AiError> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !name.contains("__");
    if valid {
        Ok(())
    } else {
        Err(AiError::from_kind(
            ErrorKind::InvalidRequest,
            format!(
                "the MCP server name {name:?} may only contain letters, digits, '-' and single '_'"
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_message_is_sent_as_is() {
        let request = ChatRequest::from_prompt("Fade the card in");
        assert_eq!(cli_prompt(&request).unwrap(), "Fade the card in");
    }

    #[test]
    fn a_resumed_session_sends_only_the_new_message() {
        let request = ChatRequest {
            messages: vec![
                ChatMessage::user("Fade the card in"),
                ChatMessage::assistant("Done."),
                ChatMessage::user("Make it slower"),
            ],
            session_id: Some("abc".to_owned()),
            ..ChatRequest::default()
        };
        assert_eq!(cli_prompt(&request).unwrap(), "Make it slower");
    }

    #[test]
    fn without_a_session_earlier_turns_are_included() {
        let request = ChatRequest {
            messages: vec![
                ChatMessage::user("Fade the card in"),
                ChatMessage::assistant("Done."),
                ChatMessage::user("Make it slower"),
            ],
            ..ChatRequest::default()
        };
        let prompt = cli_prompt(&request).unwrap();
        let first = prompt.find("[User]\nFade the card in").unwrap();
        let second = prompt.find("[Assistant]\nDone.").unwrap();
        assert!(first < second);
        assert!(prompt.ends_with("Make it slower"));
    }

    #[test]
    fn a_conversation_without_a_user_message_is_rejected() {
        let request = ChatRequest {
            messages: vec![ChatMessage::assistant("Hello")],
            ..ChatRequest::default()
        };
        let error = cli_prompt(&request).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidRequest);
        assert!(cli_prompt(&ChatRequest::default()).is_err());
    }

    #[test]
    fn working_dir_is_created_on_demand() {
        let root = tempfile::tempdir().unwrap();
        let wanted = root.path().join("a/b/workspace");
        let request = ChatRequest {
            working_dir: Some(wanted.clone()),
            ..ChatRequest::default()
        };
        assert_eq!(prepare_working_dir(&request).unwrap(), wanted);
        assert!(wanted.is_dir());
    }

    #[test]
    fn mcp_names_are_restricted() {
        assert!(validate_mcp_name("moshionly").is_ok());
        assert!(validate_mcp_name("moshionly-dev_2").is_ok());
        for bad in ["", "two words", "a__b", "quote\"", "semi;colon", "ünï"] {
            assert!(
                validate_mcp_name(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }
}
