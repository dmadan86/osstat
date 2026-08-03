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
    /// Wall-clock seconds the assistant turn took, where it was measured.
    ///
    /// `None` on a user turn, which takes as long as the person typing it, and
    /// on any assistant turn written before this field existed.
    ///
    /// `#[serde(default)]` is what makes that second case work rather than
    /// throw: conversations are files on disk that a user already has, and a
    /// missing field must read as "not measured" rather than fail the load and
    /// take the whole conversation with it.
    #[serde(default)]
    pub elapsed_seconds: Option<f64>,
    /// When this turn was recorded, in milliseconds since the Unix epoch.
    ///
    /// Milliseconds rather than a formatted string because the format belongs
    /// to whoever is reading it: the front end renders this in the viewer's own
    /// locale and time zone, which it cannot do from text this crate has
    /// already decided the shape of.
    ///
    /// `None` on any turn written before this field existed, and `#[serde(default)]`
    /// for exactly the reason [`Self::elapsed_seconds`] carries it. Also `None`
    /// if the system clock is set before 1970 or past the year 292 million,
    /// which is a clock problem rather than a reason to refuse to save a
    /// message.
    ///
    /// Declared to the bindings as `number`, as every other 64-bit field here
    /// is: `ts-rs` would otherwise write `bigint`, which `JSON.parse` never
    /// produces, so the type would describe something the front end can never
    /// receive. A millisecond count stays exact in a double until the year
    /// 287396.
    #[serde(default)]
    #[cfg_attr(feature = "ts-bindings", ts(type = "number | null"))]
    pub sent_at: Option<i64>,
}

/// The current time in milliseconds since the Unix epoch, if the clock allows.
///
/// `None` rather than a panic or a zero on a clock set before 1970: a machine
/// with a wrong clock still gets to keep its conversations, and a missing
/// timestamp renders as no timestamp, which is honest. Zero would render as
/// "01:00" and claim something the clock never said.
#[must_use]
pub fn now_millis() -> Option<i64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_millis()).ok())
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
                elapsed_seconds: None,
                sent_at: None,
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
    fn a_conversation_written_before_timestamps_existed_still_loads() {
        // The same guarantee `a_conversation_written_before_response_times_existed_still_loads`
        // makes, for the field added after it. Written by hand for the same
        // reason: a fixture built from today's `Message` could not be missing
        // `sentAt`, so it could not prove anything about the files on disk.
        //
        // This one carries `elapsedSeconds` and not `sentAt`, which is the
        // shape a conversation held between the two changes actually has --
        // the interesting case, and the one a fixture missing both fields
        // would not exercise.
        let root = tempfile::tempdir().unwrap();
        let store = ConversationStore::new(root.path().to_path_buf());
        std::fs::create_dir_all(store.directory()).unwrap();
        std::fs::write(
            store.directory().join("mid.json"),
            r#"{
              "id": "mid",
              "title": "About tea",
              "modelName": "llama-7b",
              "messages": [
                {
                  "role": "user",
                  "content": "how long do you steep it",
                  "usage": null,
                  "stopped": false,
                  "elapsedSeconds": null
                },
                {
                  "role": "assistant",
                  "content": "three minutes",
                  "usage": { "promptTokens": 44, "completionTokens": 48 },
                  "stopped": false,
                  "elapsedSeconds": 6.25
                }
              ]
            }"#,
        )
        .unwrap();

        let loaded = store.load("mid").unwrap();

        // What was on disk survived, and the new field reads as "not recorded"
        // rather than as the epoch -- a message sent in 1970 is a claim the
        // file never made, and the UI draws nothing rather than "01:00".
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[1].content, "three minutes");
        assert_eq!(loaded.messages[1].elapsed_seconds, Some(6.25));
        assert!(loaded.messages[0].sent_at.is_none());
        assert!(loaded.messages[1].sent_at.is_none());
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn a_timestamp_survives_the_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let store = ConversationStore::new(root.path().to_path_buf());
        let mut stamped = conversation("abc");
        // 2026-08-03T09:15:00Z, a fixed instant rather than `now_millis()`, so
        // the assertion is about storage and not about the clock.
        stamped.messages[0].sent_at = Some(1_785_921_300_000);

        store.save(&stamped).unwrap();

        assert_eq!(
            store.load("abc").unwrap().messages[0].sent_at,
            Some(1_785_921_300_000)
        );
    }

    #[test]
    fn the_clock_reads_as_a_plausible_recent_instant() {
        // Guards the unit rather than the clock: seconds instead of
        // milliseconds would put every message in 1970 and render a whole
        // transcript at the same wrong time.
        let now = now_millis().expect("the system clock is after 1970");

        // 2020-01-01 and 2100-01-01 in milliseconds.
        assert!(now > 1_577_836_800_000, "{now} is implausibly early");
        assert!(now < 4_102_444_800_000, "{now} is implausibly late");
    }

    #[test]
    fn a_conversation_written_before_response_times_existed_still_loads() {
        // The one that cannot be allowed to regress. Adding a field to
        // `Message` changes an on-disk format that users already have files
        // in, and a load that rejected them would lose real conversations to
        // gain a figure beside them -- a straight downgrade.
        //
        // The JSON is written by hand rather than by serialising an older
        // struct, because what has to keep parsing is the bytes on disk, and a
        // fixture built from today's type could never be missing the field.
        let root = tempfile::tempdir().unwrap();
        let store = ConversationStore::new(root.path().to_path_buf());
        std::fs::create_dir_all(store.directory()).unwrap();
        std::fs::write(
            store.directory().join("old.json"),
            r#"{
              "id": "old",
              "title": "About tea",
              "modelName": "llama-7b",
              "messages": [
                { "role": "user", "content": "how long do you steep it", "usage": null, "stopped": false },
                {
                  "role": "assistant",
                  "content": "three minutes",
                  "usage": { "promptTokens": 44, "completionTokens": 48 },
                  "stopped": false
                }
              ]
            }"#,
        )
        .unwrap();

        let loaded = store.load("old").unwrap();

        // Everything that was there is still there, and the new field reads as
        // "not measured" rather than as zero -- a reply that took no time is a
        // claim the file never made.
        assert_eq!(loaded.title, "About tea");
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[1].content, "three minutes");
        assert_eq!(loaded.messages[1].usage.unwrap().completion_tokens, 48);
        assert!(loaded.messages[1].elapsed_seconds.is_none());
        // And it survives being listed, which is the path the UI actually
        // takes: `list` skips whatever fails to parse, so a broken load would
        // show as an empty list rather than as an error.
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn a_response_time_survives_the_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let store = ConversationStore::new(root.path().to_path_buf());
        let mut timed = conversation("abc");
        timed.messages[0].elapsed_seconds = Some(6.25);

        store.save(&timed).unwrap();

        assert_eq!(
            store.load("abc").unwrap().messages[0].elapsed_seconds,
            Some(6.25)
        );
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
