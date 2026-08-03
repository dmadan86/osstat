//! Conversations, kept as one JSON file each.
//!
//! Not a database. The directory is meant to be readable: a user can open it
//! and see exactly what osstat kept, which is the same argument ADR-012 makes
//! for pinning a hash a person can review. Deleting a conversation deletes a
//! file, with nothing left behind to explain.
//!
//! Identifiers arrive from the front end, so every one is validated before it
//! reaches a path. A traversal here would turn a chat id into a filesystem
//! read.

use crate::{ChatError, Usage};
use std::path::PathBuf;

/// Who said something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub enum Role {
    /// The standing instruction.
    System,
    /// The person.
    User,
    /// The model.
    Assistant,
}

/// One stored turn.
///
/// Distinct from [`crate::client::Message`], which is the wire shape. Token
/// counts and a stopped flag are presentation state and have no business on a
/// request body.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Who said it.
    pub role: Role,
    /// What was said.
    pub content: String,
    /// Token counts for this exchange, where the server reported them.
    pub usage: Option<Usage>,
    /// Whether generation was stopped before the model finished.
    pub stopped: bool,
}

/// One conversation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    /// Stable identifier, and the file's stem.
    pub id: String,
    /// What the list shows.
    pub title: String,
    /// The model this conversation was held with.
    pub model_name: String,
    /// Every turn, oldest first.
    pub messages: Vec<Message>,
}

/// The directory conversations live in.
#[derive(Debug, Clone)]
pub struct ConversationStore {
    root: PathBuf,
}

impl ConversationStore {
    /// Creates a store rooted at `root`, typically the Tauri app-data directory.
    #[must_use]
    pub const fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The directory holding every conversation file.
    #[must_use]
    pub fn directory(&self) -> PathBuf {
        self.root.join("conversations")
    }

    /// Rejects any identifier that could reach outside the directory.
    ///
    /// Allowing only this alphabet is what makes the join below safe, rather
    /// than trying to detect traversal after the fact.
    fn path_for(&self, id: &str) -> Result<PathBuf, ChatError> {
        let usable = !id.is_empty()
            && id.len() <= 64
            && id
                .chars()
                .all(|unit| unit.is_ascii_alphanumeric() || unit == '-' || unit == '_');

        if usable {
            Ok(self.directory().join(format!("{id}.json")))
        } else {
            Err(ChatError::BadChunk(format!(
                "{id:?} is not a usable conversation id"
            )))
        }
    }

    /// Every readable conversation, oldest file first.
    ///
    /// A file that does not parse is skipped rather than failing the list. The
    /// directory is user-visible by design, so something else will end up in it.
    #[must_use]
    pub fn list(&self) -> Vec<Conversation> {
        let Ok(entries) = std::fs::read_dir(self.directory()) else {
            return Vec::new();
        };

        let mut found: Vec<Conversation> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
            .filter_map(|text| serde_json::from_str(&text).ok())
            .collect();

        found.sort_by(|left, right| left.id.cmp(&right.id));
        found
    }

    /// Reads one conversation.
    ///
    /// # Errors
    ///
    /// [`ChatError::BadChunk`] for an unusable id or unreadable contents,
    /// [`ChatError::Io`] if the file cannot be read.
    pub fn load(&self, id: &str) -> Result<Conversation, ChatError> {
        let text = std::fs::read_to_string(self.path_for(id)?)?;
        serde_json::from_str(&text).map_err(|error| ChatError::BadChunk(error.to_string()))
    }

    /// Writes one conversation, creating the directory if needed.
    ///
    /// # Errors
    ///
    /// [`ChatError::BadChunk`] for an unusable id, [`ChatError::Io`] on a write
    /// failure.
    pub fn save(&self, conversation: &Conversation) -> Result<(), ChatError> {
        let path = self.path_for(&conversation.id)?;
        std::fs::create_dir_all(self.directory())?;
        let text = serde_json::to_string_pretty(conversation)
            .map_err(|error| ChatError::BadChunk(error.to_string()))?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Removes one conversation's file.
    ///
    /// # Errors
    ///
    /// [`ChatError::BadChunk`] for an unusable id, [`ChatError::Io`] if the
    /// file exists but cannot be removed.
    pub fn delete(&self, id: &str) -> Result<(), ChatError> {
        let path = self.path_for(id)?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ChatError::Io(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn conversation(id: &str) -> Conversation {
        Conversation {
            id: id.to_owned(),
            title: "About tea".to_owned(),
            model_name: "llama-7b".to_owned(),
            messages: vec![Message {
                role: Role::User,
                content: "hello".to_owned(),
                usage: None,
                stopped: false,
            }],
        }
    }

    #[test]
    fn a_saved_conversation_comes_back() {
        let root = tempfile::tempdir().unwrap();
        let store = ConversationStore::new(root.path().to_path_buf());

        store.save(&conversation("abc")).unwrap();

        assert_eq!(store.load("abc").unwrap(), conversation("abc"));
    }

    #[test]
    fn deleting_a_conversation_removes_its_file() {
        // The delete control has to actually erase. A store that merely hid the
        // conversation would make the UI a lie about what is on disk.
        let root = tempfile::tempdir().unwrap();
        let store = ConversationStore::new(root.path().to_path_buf());
        store.save(&conversation("abc")).unwrap();

        store.delete("abc").unwrap();

        assert!(store.load("abc").is_err());
        assert!(
            std::fs::read_dir(store.directory()).map_or(true, |entries| entries.count() == 0),
            "a file survived the delete"
        );
    }

    #[test]
    fn listing_skips_files_that_are_not_conversations() {
        // The directory is user-visible by design, so something else will end
        // up in it eventually. That must not break the list.
        let root = tempfile::tempdir().unwrap();
        let store = ConversationStore::new(root.path().to_path_buf());
        store.save(&conversation("abc")).unwrap();
        std::fs::create_dir_all(store.directory()).unwrap();
        std::fs::write(store.directory().join("notes.txt"), "hello").unwrap();
        std::fs::write(store.directory().join("broken.json"), "{{{").unwrap();

        let found = store.list();

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "abc");
    }

    #[test]
    fn an_id_cannot_escape_the_conversation_directory() {
        // The id reaches this from the front end. A traversal would let a
        // crafted id read or delete a file elsewhere on disk.
        let root = tempfile::tempdir().unwrap();
        let store = ConversationStore::new(root.path().to_path_buf());

        for hostile in ["../escape", "..\\escape", "a/b", "", "."] {
            assert!(
                store.load(hostile).is_err(),
                "{hostile:?} was accepted as an id"
            );
            assert!(store.delete(hostile).is_err());
        }
    }

    #[test]
    fn token_counts_survive_the_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let store = ConversationStore::new(root.path().to_path_buf());
        let mut with_usage = conversation("abc");
        with_usage.messages[0].usage = Some(crate::Usage {
            prompt_tokens: 44,
            completion_tokens: 48,
        });

        store.save(&with_usage).unwrap();

        assert_eq!(
            store.load("abc").unwrap().messages[0]
                .usage
                .unwrap()
                .prompt_tokens,
            44
        );
    }
}
