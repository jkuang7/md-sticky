use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tauri_plugin_log::log;
use tauri_plugin_store::StoreExt;
use uuid::Uuid;

use crate::{
    settings::MenuSettings,
    windows::{open_sticky, reflow_linked_stack},
};

const BACKUP_FOLDER: &str = "backups";
const NOTES_DATA: &str = "notes.json";
const PREVIOUS_NOTES_DATA: &str = "notes.previous.json";
const SETTINGS: &str = "settings";
const STORAGE_VERSION: u32 = 1;
static TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone, PartialEq)]
pub struct StoredNote {
    pub id: String,
    pub document: Value,
    pub color: String,
    pub x: i32,
    pub y: i32,
    pub expanded_height: u32,
    pub expanded_width: u32,
    pub collapsed: bool,
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<u64>,
}

impl StoredNote {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            document: empty_document(),
            color: "#fff9b1".into(),
            x: 0,
            y: 0,
            expanded_height: 250,
            expanded_width: 300,
            collapsed: false,
            pinned: false,
            closed_at: None,
        }
    }

    fn recovery_notice(restored_previous: bool) -> Self {
        let outcome = if restored_previous {
            "The last valid snapshot was restored."
        } else {
            "No valid snapshot was available, so a fresh note store was created."
        };
        let mut note = Self::new();
        note.color = "#e1a1b1".into();
        note.document = json!({
            "type": "doc",
            "content": [
                {
                    "type": "heading",
                    "attrs": { "level": 2 },
                    "content": [{ "type": "text", "text": "Recovery notice" }]
                },
                {
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": format!(
                            "Sticky found unreadable note data and preserved the exact damaged files in its backups folder. {outcome}"
                        )
                    }]
                }
            ]
        });
        note
    }
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone, PartialEq)]
struct NoteStore {
    version: u32,
    notes: BTreeMap<String, StoredNote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    linked_stack: Option<Vec<String>>,
}

impl NoteStore {
    fn empty() -> Self {
        Self {
            version: STORAGE_VERSION,
            notes: BTreeMap::new(),
            linked_stack: None,
        }
    }

    fn brand_new() -> Self {
        let mut store = Self::empty();
        let note = StoredNote::new();
        store.notes.insert(note.id.clone(), note);
        store
    }

    fn add_recovery_notice(&mut self, restored_previous: bool) {
        let note = StoredNote::recovery_notice(restored_previous);
        if let Some(order) = &mut self.linked_stack {
            order.push(note.id.clone());
        }
        self.notes.insert(note.id.clone(), note);
    }

    fn ordered_notes(&self) -> Vec<StoredNote> {
        let Some(order) = &self.linked_stack else {
            return self.notes.values().cloned().collect();
        };
        order
            .iter()
            .filter_map(|id| self.notes.get(id).cloned())
            .collect()
    }
}

fn empty_document() -> Value {
    json!({ "type": "doc", "content": [{ "type": "paragraph" }] })
}

pub struct NoteRepository {
    path: PathBuf,
    previous_path: PathBuf,
    notes: Mutex<NoteStore>,
}

impl NoteRepository {
    pub fn load(app: &AppHandle) -> anyhow::Result<Self> {
        let app_data_dir = app
            .path()
            .app_data_dir()
            .context("Failed to get app data directory")?;
        Self::load_from_dir(&app_data_dir)
    }

    fn load_from_dir(app_data_dir: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(app_data_dir).context("Failed to create app data directory")?;
        let path = app_data_dir.join(NOTES_DATA);
        let previous_path = app_data_dir.join(PREVIOUS_NOTES_DATA);

        let store = if path.exists() {
            let current_bytes = fs::read(&path).context("Failed to read note storage")?;
            match parse_store(&current_bytes) {
                Ok(store) => store,
                Err(current_error) => recover_store(
                    app_data_dir,
                    &path,
                    &previous_path,
                    current_bytes,
                    current_error,
                )?,
            }
        } else if previous_path.exists() {
            recover_without_current(app_data_dir, &path, &previous_path)?
        } else {
            let store = NoteStore::brand_new();
            persist_store(&path, &previous_path, &store, false)?;
            store
        };

        Ok(Self {
            path,
            previous_path,
            notes: Mutex::new(store),
        })
    }

    pub fn all(&self) -> anyhow::Result<Vec<StoredNote>> {
        let notes = self
            .notes
            .lock()
            .map_err(|_| anyhow::anyhow!("Note storage lock poisoned"))?;
        Ok(notes.ordered_notes())
    }

    pub fn active(&self) -> anyhow::Result<Vec<StoredNote>> {
        Ok(self
            .all()?
            .into_iter()
            .filter(|note| note.closed_at.is_none())
            .collect())
    }

    pub fn get(&self, id: &str) -> anyhow::Result<StoredNote> {
        let notes = self
            .notes
            .lock()
            .map_err(|_| anyhow::anyhow!("Note storage lock poisoned"))?;
        notes
            .notes
            .get(id)
            .cloned()
            .with_context(|| format!("Unknown note id {id}"))
    }

    pub fn create(&self) -> anyhow::Result<StoredNote> {
        self.create_at(0, 0)
    }

    pub fn create_at(&self, x: i32, y: i32) -> anyhow::Result<StoredNote> {
        let mut note = StoredNote::new();
        note.x = x;
        note.y = y;
        let result = note.clone();
        self.mutate(|store| {
            if let Some(order) = &mut store.linked_stack {
                order.push(note.id.clone());
            }
            store.notes.insert(note.id.clone(), note);
            Ok(())
        })?;
        Ok(result)
    }

    pub fn update<F>(&self, id: &str, update: F) -> anyhow::Result<StoredNote>
    where
        F: FnOnce(&mut StoredNote) -> anyhow::Result<()>,
    {
        let mut result = None;
        self.mutate(|store| {
            let note = store
                .notes
                .get_mut(id)
                .with_context(|| format!("Unknown note id {id}"))?;
            update(note)?;
            result = Some(note.clone());
            Ok(())
        })?;
        result.context("Note update did not produce a result")
    }

    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.mutate(|store| {
            store.notes.remove(id);
            if let Some(order) = &mut store.linked_stack {
                order.retain(|note_id| note_id != id);
            }
            Ok(())
        })
    }

    pub fn linked_stack(&self) -> anyhow::Result<Option<Vec<String>>> {
        let notes = self
            .notes
            .lock()
            .map_err(|_| anyhow::anyhow!("Note storage lock poisoned"))?;
        Ok(notes.linked_stack.clone())
    }

    pub fn set_linked_stack(&self, order: Option<Vec<String>>) -> anyhow::Result<()> {
        self.mutate(|store| {
            store.linked_stack = order;
            Ok(())
        })
    }

    pub fn close(&self, id: &str) -> anyhow::Result<StoredNote> {
        let closed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System time is before UNIX epoch")?
            .as_millis()
            .try_into()
            .context("System timestamp did not fit in note storage")?;
        self.update(id, |note| {
            note.closed_at = Some(closed_at);
            Ok(())
        })
    }

    pub fn restore_last_closed(&self) -> anyhow::Result<Option<StoredNote>> {
        let id = self
            .all()?
            .into_iter()
            .filter_map(|note| note.closed_at.map(|closed_at| (closed_at, note.id)))
            .max()
            .map(|(_, id)| id);
        let Some(id) = id else {
            return Ok(None);
        };
        self.update(&id, |note| {
            note.closed_at = None;
            Ok(())
        })
        .map(Some)
    }

    fn mutate<F>(&self, update: F) -> anyhow::Result<()>
    where
        F: FnOnce(&mut NoteStore) -> anyhow::Result<()>,
    {
        let mut guard = self
            .notes
            .lock()
            .map_err(|_| anyhow::anyhow!("Note storage lock poisoned"))?;
        let mut candidate = guard.clone();
        update(&mut candidate)?;
        validate_store(&candidate)?;
        persist_store(&self.path, &self.previous_path, &candidate, true)?;
        *guard = candidate;
        Ok(())
    }
}

fn parse_store(bytes: &[u8]) -> anyhow::Result<NoteStore> {
    let store: NoteStore = serde_json::from_slice(bytes).context("Failed to parse note storage")?;
    validate_store(&store)?;
    Ok(store)
}

fn validate_store(store: &NoteStore) -> anyhow::Result<()> {
    if store.version != STORAGE_VERSION {
        bail!(
            "Unsupported note storage version {} (expected {})",
            store.version,
            STORAGE_VERSION
        );
    }

    for (key, note) in &store.notes {
        if key != &note.id {
            bail!("Note map key did not match the stored note id");
        }
        if note.document.get("type").and_then(Value::as_str) != Some("doc") {
            bail!("Note {} did not contain a Tiptap document", note.id);
        }
    }
    if let Some(order) = &store.linked_stack {
        let unique: std::collections::BTreeSet<_> = order.iter().collect();
        if unique.len() != order.len() {
            bail!("Linked note stack contained duplicate note ids");
        }
        if order.len() != store.notes.len() || order.iter().any(|id| !store.notes.contains_key(id))
        {
            bail!("Linked note stack did not contain every stored note exactly once");
        }
    }
    Ok(())
}

fn recover_store(
    app_data_dir: &Path,
    path: &Path,
    previous_path: &Path,
    current_bytes: Vec<u8>,
    current_error: anyhow::Error,
) -> anyhow::Result<NoteStore> {
    let current_backup = preserve_damaged_bytes(app_data_dir, NOTES_DATA, &current_bytes)?;
    log::error!(
        "Current note store is unreadable ({current_error:#}); preserved it at {:?}",
        current_backup
    );

    let (mut store, restored_previous) = if previous_path.exists() {
        let previous_bytes =
            fs::read(previous_path).context("Failed to read previous note snapshot")?;
        match parse_store(&previous_bytes) {
            Ok(store) => (store, true),
            Err(previous_error) => {
                let previous_backup =
                    preserve_damaged_bytes(app_data_dir, PREVIOUS_NOTES_DATA, &previous_bytes)?;
                log::error!(
                    "Previous note snapshot is unreadable ({previous_error:#}); preserved it at {:?}",
                    previous_backup
                );
                (NoteStore::empty(), false)
            }
        }
    } else {
        (NoteStore::empty(), false)
    };

    store.add_recovery_notice(restored_previous);
    validate_store(&store)?;
    persist_store(path, previous_path, &store, false)?;
    Ok(store)
}

fn recover_without_current(
    app_data_dir: &Path,
    path: &Path,
    previous_path: &Path,
) -> anyhow::Result<NoteStore> {
    let previous_bytes =
        fs::read(previous_path).context("Failed to read previous note snapshot")?;
    let (mut store, restored_previous) = match parse_store(&previous_bytes) {
        Ok(store) => (store, true),
        Err(previous_error) => {
            let previous_backup =
                preserve_damaged_bytes(app_data_dir, PREVIOUS_NOTES_DATA, &previous_bytes)?;
            log::error!(
                "Previous note snapshot is unreadable ({previous_error:#}); preserved it at {:?}",
                previous_backup
            );
            (NoteStore::empty(), false)
        }
    };
    store.add_recovery_notice(restored_previous);
    persist_store(path, previous_path, &store, false)?;
    Ok(store)
}

fn preserve_damaged_bytes(
    app_data_dir: &Path,
    source_name: &str,
    bytes: &[u8],
) -> anyhow::Result<PathBuf> {
    let backup_dir = app_data_dir.join(BACKUP_FOLDER);
    fs::create_dir_all(&backup_dir).context("Failed to create backup directory")?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System time is before UNIX epoch")?
        .as_millis();
    let backup_path = backup_dir.join(format!(
        "corrupt-{source_name}-{timestamp}-{}.json",
        Uuid::new_v4()
    ));
    write_bytes(&backup_path, bytes, true)?;
    sync_directory(&backup_dir);
    Ok(backup_path)
}

fn persist_store(
    path: &Path,
    previous_path: &Path,
    store: &NoteStore,
    rotate_current: bool,
) -> anyhow::Result<()> {
    let parent = path.parent().context("Note storage path has no parent")?;
    fs::create_dir_all(parent).context("Failed to create note storage directory")?;
    let bytes = serde_json::to_vec_pretty(store).context("Failed to serialize note storage")?;
    let new_path = temporary_path(parent, NOTES_DATA);

    let result = (|| -> anyhow::Result<()> {
        write_bytes(&new_path, &bytes, true).context("Failed to write temporary note storage")?;

        if rotate_current && path.exists() {
            let current_bytes = fs::read(path).context("Failed to read current note snapshot")?;
            parse_store(&current_bytes).context("Refusing to rotate an invalid current store")?;
            let previous_temp = temporary_path(parent, PREVIOUS_NOTES_DATA);
            let rotate_result = (|| -> anyhow::Result<()> {
                write_bytes(&previous_temp, &current_bytes, true)
                    .context("Failed to write previous note snapshot")?;
                fs::rename(&previous_temp, previous_path)
                    .context("Failed to atomically replace previous note snapshot")?;
                Ok(())
            })();
            if rotate_result.is_err() {
                let _ = fs::remove_file(&previous_temp);
            }
            rotate_result?;
        }

        fs::rename(&new_path, path).context("Failed to atomically replace note storage")?;
        sync_directory(parent);
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&new_path);
    }
    result
}

fn temporary_path(parent: &Path, name: &str) -> PathBuf {
    let temp_id = TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(".{name}.tmp-{}-{temp_id}", std::process::id()))
}

fn write_bytes(path: &Path, bytes: &[u8], create_new: bool) -> anyhow::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true);
    if create_new {
        options.create_new(true);
    } else {
        options.create(true).truncate(true);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn sync_directory(path: &Path) {
    if let Ok(directory) = OpenOptions::new().read(true).open(path) {
        let _ = directory.sync_all();
    }
}

pub fn note_id_from_label(label: &str) -> anyhow::Result<&str> {
    label
        .strip_prefix("sticky_")
        .context("Window label did not contain a note id")
}

pub fn load_stickies(app: &AppHandle) -> anyhow::Result<()> {
    let repository = app.state::<NoteRepository>();
    let active = repository.active()?;
    for note in &active {
        if let Err(error) = open_sticky(app, &note) {
            log::error!("Could not open note {}: {error:#}", note.id);
        }
    }
    reflow_linked_stack(app)?;
    if let Some(note) = active.iter().find(|note| !note.collapsed) {
        if let Some(window) = app.get_webview_window(&format!("sticky_{}", note.id)) {
            window.set_focus()?;
        }
    }
    Ok(())
}

pub fn load_settings(app: &AppHandle) -> anyhow::Result<MenuSettings> {
    log::info!("Loading settings");

    let store = app.store(SETTINGS)?;
    let bring_to_front = store
        .get("bring_to_front")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    let autostart = store
        .get("autostart")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    MenuSettings::new(app, bring_to_front, autostart)
}

pub fn save_settings(app: &AppHandle) -> anyhow::Result<()> {
    log::info!("Saving settings");

    let store = app.store(SETTINGS)?;
    let settings = app.state::<MenuSettings>();
    store.set("bring_to_front", settings.bring_to_front()?);
    store.set("autostart", settings.autostart()?);
    store.save()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread};

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("md-sticky-{name}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn cleanup(path: PathBuf) {
        fs::remove_dir_all(path).unwrap();
    }

    fn document(text: &str) -> Value {
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": text }]
            }]
        })
    }

    fn backup_bytes(dir: &Path) -> Vec<Vec<u8>> {
        let mut backups: Vec<_> = fs::read_dir(dir.join(BACKUP_FOLDER))
            .unwrap()
            .map(|entry| fs::read(entry.unwrap().path()).unwrap())
            .collect();
        backups.sort();
        backups
    }

    #[test]
    fn fresh_store_initializes_with_one_empty_note() {
        let dir = temp_dir("fresh");
        let repository = NoteRepository::load_from_dir(&dir).unwrap();
        let notes = repository.all().unwrap();

        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].document, empty_document());
        assert!(dir.join(NOTES_DATA).is_file());
        assert!(!dir.join(PREVIOUS_NOTES_DATA).exists());
        cleanup(dir);
    }

    #[test]
    fn intentional_empty_store_remains_empty_after_restart() {
        let dir = temp_dir("intentional-empty");
        let repository = NoteRepository::load_from_dir(&dir).unwrap();
        let id = repository.all().unwrap()[0].id.clone();
        repository.delete(&id).unwrap();

        let reloaded = NoteRepository::load_from_dir(&dir).unwrap();
        assert!(reloaded.all().unwrap().is_empty());
        cleanup(dir);
    }

    #[test]
    fn closed_note_is_retained_but_hidden_after_restart() {
        let dir = temp_dir("closed-retained");
        let repository = NoteRepository::load_from_dir(&dir).unwrap();
        let note = repository.all().unwrap()[0].clone();
        repository
            .update(&note.id, |note| {
                note.document = document("must survive close");
                Ok(())
            })
            .unwrap();
        repository.close(&note.id).unwrap();

        let reloaded = NoteRepository::load_from_dir(&dir).unwrap();
        assert!(reloaded.active().unwrap().is_empty());
        let retained = reloaded.get(&note.id).unwrap();
        assert!(retained.closed_at.is_some());
        assert_eq!(
            retained.document["content"][0]["content"][0]["text"],
            "must survive close"
        );
        cleanup(dir);
    }

    #[test]
    fn restore_last_closed_restores_only_the_most_recent_note() {
        let dir = temp_dir("restore-last-closed");
        let repository = NoteRepository::load_from_dir(&dir).unwrap();
        let first = repository.all().unwrap()[0].clone();
        let second = repository.create().unwrap();
        repository
            .update(&first.id, |note| {
                note.closed_at = Some(1);
                Ok(())
            })
            .unwrap();
        repository
            .update(&second.id, |note| {
                note.closed_at = Some(2);
                Ok(())
            })
            .unwrap();

        let restored = repository.restore_last_closed().unwrap().unwrap();
        assert_eq!(restored.id, second.id);
        assert!(restored.closed_at.is_none());
        assert!(repository.get(&first.id).unwrap().closed_at.is_some());
        assert_eq!(repository.active().unwrap(), vec![restored]);
        cleanup(dir);
    }

    #[test]
    fn linked_order_survives_close_restore_and_appends_new_notes() {
        let dir = temp_dir("linked-order-lifecycle");
        let repository = NoteRepository::load_from_dir(&dir).unwrap();
        let first = repository.all().unwrap()[0].clone();
        let second = repository.create().unwrap();
        let third = repository.create().unwrap();
        repository
            .set_linked_stack(Some(vec![
                third.id.clone(),
                first.id.clone(),
                second.id.clone(),
            ]))
            .unwrap();

        repository.close(&first.id).unwrap();
        assert_eq!(
            repository
                .active()
                .unwrap()
                .into_iter()
                .map(|note| note.id)
                .collect::<Vec<_>>(),
            vec![third.id.clone(), second.id.clone()]
        );
        repository.restore_last_closed().unwrap();
        assert_eq!(
            repository
                .active()
                .unwrap()
                .into_iter()
                .map(|note| note.id)
                .collect::<Vec<_>>(),
            vec![third.id.clone(), first.id.clone(), second.id.clone()]
        );

        let fourth = repository.create().unwrap();
        assert_eq!(
            repository.linked_stack().unwrap().unwrap(),
            vec![third.id, first.id, second.id, fourth.id]
        );
        let reloaded = NoteRepository::load_from_dir(&dir).unwrap();
        assert_eq!(
            reloaded.linked_stack().unwrap(),
            repository.linked_stack().unwrap()
        );
        cleanup(dir);
    }

    #[test]
    fn interrupted_temporary_write_does_not_replace_the_store() {
        let dir = temp_dir("interrupted");
        let repository = NoteRepository::load_from_dir(&dir).unwrap();
        let note = repository.all().unwrap()[0].clone();
        fs::write(dir.join(".notes.json.tmp-interrupted"), b"{").unwrap();

        let reloaded = NoteRepository::load_from_dir(&dir).unwrap();
        assert_eq!(reloaded.get(&note.id).unwrap(), note);
        cleanup(dir);
    }

    #[test]
    fn concurrent_saves_to_different_notes_do_not_drop_updates() {
        let dir = temp_dir("concurrent");
        let repository = Arc::new(NoteRepository::load_from_dir(&dir).unwrap());
        let notes: Vec<_> = (0..12).map(|_| repository.create().unwrap()).collect();
        let handles: Vec<_> = notes
            .iter()
            .enumerate()
            .map(|(index, note)| {
                let repository = Arc::clone(&repository);
                let id = note.id.clone();
                thread::spawn(move || {
                    repository
                        .update(&id, |note| {
                            note.document = document(&format!("note {index}"));
                            Ok(())
                        })
                        .unwrap();
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        let reloaded = NoteRepository::load_from_dir(&dir).unwrap();
        for (index, note) in notes.iter().enumerate() {
            assert_eq!(
                reloaded.get(&note.id).unwrap().document["content"][0]["content"][0]["text"],
                format!("note {index}")
            );
        }
        cleanup(dir);
    }

    #[test]
    fn each_save_rotates_the_last_valid_current_store() {
        let dir = temp_dir("rotation");
        let repository = NoteRepository::load_from_dir(&dir).unwrap();
        let note = repository.all().unwrap()[0].clone();
        repository
            .update(&note.id, |note| {
                note.document = document("first");
                Ok(())
            })
            .unwrap();
        let first_current = fs::read(dir.join(NOTES_DATA)).unwrap();

        repository
            .update(&note.id, |note| {
                note.document = document("second");
                Ok(())
            })
            .unwrap();

        assert_eq!(
            fs::read(dir.join(PREVIOUS_NOTES_DATA)).unwrap(),
            first_current
        );
        assert_eq!(
            NoteRepository::load_from_dir(&dir)
                .unwrap()
                .get(&note.id)
                .unwrap()
                .document["content"][0]["content"][0]["text"],
            "second"
        );
        cleanup(dir);
    }

    #[test]
    fn corrupt_current_restores_previous_and_adds_notice() {
        let dir = temp_dir("restore");
        let repository = NoteRepository::load_from_dir(&dir).unwrap();
        let note = repository.all().unwrap()[0].clone();
        repository
            .update(&note.id, |note| {
                note.document = document("saved snapshot");
                Ok(())
            })
            .unwrap();
        repository
            .update(&note.id, |note| {
                note.document = document("new current");
                Ok(())
            })
            .unwrap();
        let damaged = b"{damaged current\0bytes}".to_vec();
        fs::write(dir.join(NOTES_DATA), &damaged).unwrap();

        let recovered = NoteRepository::load_from_dir(&dir).unwrap();
        let notes = recovered.all().unwrap();
        assert_eq!(notes.len(), 2);
        assert!(notes
            .iter()
            .any(|note| note.document.to_string().contains("saved snapshot")));
        assert!(notes
            .iter()
            .any(|note| note.document.to_string().contains("Recovery notice")));
        assert_eq!(backup_bytes(&dir), vec![damaged]);
        cleanup(dir);
    }

    #[test]
    fn missing_previous_snapshot_starts_fresh_with_notice() {
        let dir = temp_dir("missing-previous");
        let damaged = b"not json at all".to_vec();
        fs::write(dir.join(NOTES_DATA), &damaged).unwrap();

        let recovered = NoteRepository::load_from_dir(&dir).unwrap();
        let notes = recovered.all().unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].document.to_string().contains("Recovery notice"));
        assert!(notes[0].document.to_string().contains("No valid snapshot"));
        assert_eq!(backup_bytes(&dir), vec![damaged]);
        cleanup(dir);
    }

    #[test]
    fn missing_current_store_restores_the_previous_snapshot() {
        let dir = temp_dir("missing-current");
        let repository = NoteRepository::load_from_dir(&dir).unwrap();
        let note = repository.all().unwrap()[0].clone();
        repository
            .update(&note.id, |note| {
                note.document = document("previous survives");
                Ok(())
            })
            .unwrap();
        repository
            .update(&note.id, |note| {
                note.document = document("current disappears");
                Ok(())
            })
            .unwrap();
        fs::remove_file(dir.join(NOTES_DATA)).unwrap();

        let recovered = NoteRepository::load_from_dir(&dir).unwrap();
        let notes = recovered.all().unwrap();
        assert_eq!(notes.len(), 2);
        assert!(notes
            .iter()
            .any(|note| note.document.to_string().contains("previous survives")));
        assert!(notes
            .iter()
            .any(|note| note.document.to_string().contains("Recovery notice")));
        cleanup(dir);
    }

    #[test]
    fn invalid_previous_snapshot_is_also_preserved_exactly() {
        let dir = temp_dir("invalid-previous");
        let damaged_current = b"current\0damaged".to_vec();
        let damaged_previous = b"previous\xffdamaged".to_vec();
        fs::write(dir.join(NOTES_DATA), &damaged_current).unwrap();
        fs::write(dir.join(PREVIOUS_NOTES_DATA), &damaged_previous).unwrap();

        let recovered = NoteRepository::load_from_dir(&dir).unwrap();
        assert_eq!(recovered.all().unwrap().len(), 1);
        assert_eq!(backup_bytes(&dir), {
            let mut expected = vec![damaged_current, damaged_previous];
            expected.sort();
            expected
        });
        cleanup(dir);
    }
}
