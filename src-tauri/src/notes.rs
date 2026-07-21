use std::{
    cmp::Reverse,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tempfile::Builder as TempFileBuilder;
use uuid::Uuid;

const NOTES_SCHEMA_VERSION: u8 = 1;
const MAX_NOTES: usize = 10_000;
const MAX_TITLE_BYTES: usize = 512;
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub body: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteInput {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NotesDocument {
    schema_version: u8,
    notes: Vec<Note>,
}

impl Default for NotesDocument {
    fn default() -> Self {
        Self {
            schema_version: NOTES_SCHEMA_VERSION,
            notes: Vec::new(),
        }
    }
}

pub struct NoteStore {
    path: PathBuf,
    document: Mutex<NotesDocument>,
}

impl NoteStore {
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let document = load_document(&path)?;
        Ok(Self {
            path,
            document: Mutex::new(document),
        })
    }

    pub fn list(&self) -> Result<Vec<Note>, String> {
        let mut notes = self
            .document
            .lock()
            .map_err(|_| "notes are temporarily unavailable".to_string())?
            .notes
            .clone();
        notes.sort_by_key(|note| Reverse(note.updated_at_ms));
        Ok(notes)
    }

    pub fn get(&self, id: &str) -> Result<Note, String> {
        validate_id(id)?;
        self.document
            .lock()
            .map_err(|_| "notes are temporarily unavailable".to_string())?
            .notes
            .iter()
            .find(|note| note.id == id)
            .cloned()
            .ok_or_else(|| "that note no longer exists".into())
    }

    pub fn create(&self, input: NoteInput) -> Result<Note, String> {
        let input = validate_input(input)?;
        let mut document = self
            .document
            .lock()
            .map_err(|_| "notes are temporarily unavailable".to_string())?;
        if document.notes.len() >= MAX_NOTES {
            return Err("remove a note before creating another one".into());
        }
        let timestamp = now_ms();
        let note = Note {
            id: Uuid::new_v4().to_string(),
            title: input.title,
            body: input.body,
            created_at_ms: timestamp,
            updated_at_ms: timestamp,
        };
        let mut next = document.clone();
        next.notes.push(note.clone());
        persist_document(&self.path, &next)?;
        *document = next;
        Ok(note)
    }

    pub fn update(&self, id: &str, input: NoteInput) -> Result<Note, String> {
        validate_id(id)?;
        let input = validate_input(input)?;
        let mut document = self
            .document
            .lock()
            .map_err(|_| "notes are temporarily unavailable".to_string())?;
        let mut next = document.clone();
        let note = next
            .notes
            .iter_mut()
            .find(|note| note.id == id)
            .ok_or_else(|| "that note no longer exists".to_string())?;
        note.title = input.title;
        note.body = input.body;
        note.updated_at_ms = now_ms().max(note.updated_at_ms.saturating_add(1));
        let updated = note.clone();
        persist_document(&self.path, &next)?;
        *document = next;
        Ok(updated)
    }

    pub fn delete(&self, id: &str) -> Result<bool, String> {
        validate_id(id)?;
        let mut document = self
            .document
            .lock()
            .map_err(|_| "notes are temporarily unavailable".to_string())?;
        let mut next = document.clone();
        let before = next.notes.len();
        next.notes.retain(|note| note.id != id);
        let deleted = next.notes.len() != before;
        if deleted {
            persist_document(&self.path, &next)?;
            *document = next;
        }
        Ok(deleted)
    }
}

fn validate_id(id: &str) -> Result<(), String> {
    Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| "the note identifier is invalid".into())
}

fn validate_input(input: NoteInput) -> Result<NoteInput, String> {
    let title = input.title.trim().to_owned();
    if title.len() > MAX_TITLE_BYTES || title.chars().any(char::is_control) {
        return Err("the note title is too long or contains unsupported characters".into());
    }
    if input.body.len() > MAX_BODY_BYTES {
        return Err("the note is larger than 2 MB".into());
    }
    Ok(NoteInput {
        title: if title.is_empty() {
            "Untitled note".into()
        } else {
            title
        },
        body: input.body,
    })
}

fn load_document(path: &Path) -> Result<NotesDocument, String> {
    if !path.exists() {
        return Ok(NotesDocument::default());
    }
    let metadata =
        fs::metadata(path).map_err(|error| format!("could not inspect notes: {error}"))?;
    if !metadata.is_file() || metadata.len() > 32 * 1024 * 1024 {
        return Err("the notes file has an invalid shape".into());
    }
    let document: NotesDocument = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("could not read notes: {error}"))?,
    )
    .map_err(|error| format!("could not understand notes: {error}"))?;
    if document.schema_version != NOTES_SCHEMA_VERSION || document.notes.len() > MAX_NOTES {
        return Err("the notes file uses an unsupported format".into());
    }
    for note in &document.notes {
        validate_id(&note.id)?;
        validate_input(NoteInput {
            title: note.title.clone(),
            body: note.body.clone(),
        })?;
    }
    Ok(document)
}

fn persist_document(path: &Path, document: &NotesDocument) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "the notes location is invalid".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create notes storage: {error}"))?;
    let bytes =
        serde_json::to_vec(document).map_err(|error| format!("could not encode notes: {error}"))?;
    let mut temporary = TempFileBuilder::new()
        .prefix(".spick-notes-")
        .tempfile_in(parent)
        .map_err(|error| format!("could not prepare notes: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("could not protect notes: {error}"))?;
    }
    temporary
        .write_all(&bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| format!("could not save notes: {error}"))?;
    temporary
        .persist(path)
        .map_err(|error| format!("could not replace notes: {}", error.error))?;
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_round_trip_and_sort_by_latest_edit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.json");
        let store = NoteStore::open(path.clone()).unwrap();
        let first = store
            .create(NoteInput {
                title: "First".into(),
                body: "# Hello".into(),
            })
            .unwrap();
        let second = store
            .create(NoteInput {
                title: "Second".into(),
                body: "Body".into(),
            })
            .unwrap();
        store
            .update(
                &first.id,
                NoteInput {
                    title: "First again".into(),
                    body: "- item".into(),
                },
            )
            .unwrap();
        assert_eq!(store.list().unwrap()[0].id, first.id);
        assert!(store.delete(&second.id).unwrap());

        let reopened = NoteStore::open(path).unwrap();
        assert_eq!(reopened.list().unwrap().len(), 1);
        assert_eq!(reopened.list().unwrap()[0].body, "- item");
    }
}
