// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `contexts.json` in the data directory: the user's contexts, their member
//! records, and the active context.
//!
//! The file also names the boot of the Mac that wrote it. Window server ids
//! are valid only until the Mac restarts, so the reactor forgets the saved
//! ids when it reads a file from another boot.

use std::ffi::{c_char, c_int, c_void};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tracing::error;

use crate::actor::parked_journal::unreadable_path;
use crate::model::contexts::{CONTEXTS_FILE_VERSION, ContextId, Contexts};

/// Reads and writes `contexts.json`.
pub struct ContextsStore {
    /// Where the file is. `None` keeps the contexts in memory only.
    path: Option<PathBuf>,
}

/// What reading `contexts.json` found.
#[derive(Debug)]
pub enum Loaded {
    /// The file was read. `boot_id` names the boot that wrote it.
    Read {
        contexts: Contexts,
        boot_id: Option<String>,
    },
    /// There is no file.
    Missing,
    /// The file could not be read and was moved aside.
    Unreadable,
}

#[derive(Serialize)]
struct FileRef<'a> {
    #[serde(flatten)]
    contexts: &'a Contexts,
    #[serde(skip_serializing_if = "Option::is_none")]
    boot_id: Option<&'a str>,
}

#[derive(Deserialize)]
struct SavedBoot {
    #[serde(default)]
    boot_id: Option<String>,
}

impl ContextsStore {
    pub fn new(path: PathBuf) -> Self {
        ContextsStore { path: Some(path) }
    }

    /// A store that never reads or writes a file.
    pub fn in_memory() -> Self {
        ContextsStore { path: None }
    }

    /// Reads the file.
    ///
    /// A file that can't be read is moved to
    /// `<name>.unreadable-<unix time>.json` next to it, using `now`.
    pub fn load(&self, now: SystemTime) -> Loaded {
        let Some(path) = &self.path else {
            return Loaded::Missing;
        };
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Loaded::Missing,
            Err(err) => return move_aside(path, now, err.into()),
        };
        let parsed = serde_json::from_slice::<Contexts>(&bytes).and_then(|contexts| {
            let saved: SavedBoot = serde_json::from_slice(&bytes)?;
            Ok((contexts, saved.boot_id))
        });
        match parsed {
            Ok((contexts, boot_id)) => Loaded::Read { contexts, boot_id },
            Err(err) => move_aside(path, now, err.into()),
        }
    }

    /// Writes the contexts, replacing the file at once so a crash never
    /// leaves half a file.
    pub fn save(&self, contexts: &Contexts, boot_id: Option<&str>) -> io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let json = serde_json::to_string_pretty(&FileRef { contexts, boot_id })?;
        let dir = path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir)?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(json.as_bytes())?;
        tmp.persist(path)?;
        Ok(())
    }
}

fn move_aside(path: &Path, now: SystemTime, err: anyhow::Error) -> Loaded {
    let aside = unreadable_path(path, now);
    match fs::rename(path, &aside) {
        Ok(()) => error!(
            ?path,
            ?aside,
            "Could not read the contexts ({err}); moved the file aside"
        ),
        Err(rename_err) => error!(
            ?path,
            ?aside,
            "Could not read the contexts ({err}), and could not move the file aside: \
             {rename_err}"
        ),
    }
    Loaded::Unreadable
}

/// Contexts with none defined whose next id comes after `after`, so a new
/// context doesn't take the id of a context whose layouts are still saved.
pub fn empty_contexts_after(after: Option<ContextId>) -> Contexts {
    let next_id = after.map_or(1, |id| id.get().saturating_add(1));
    serde_json::from_value(serde_json::json!({
        "version": CONTEXTS_FILE_VERSION,
        "next_id": next_id,
    }))
    .expect("an empty contexts file loads")
}

/// Names the current boot of the Mac by the time it booted, from
/// `kern.boottime`. `None` if that can't be read.
pub fn boot_id() -> Option<String> {
    #[repr(C)]
    struct Timeval {
        tv_sec: i64,
        tv_usec: i32,
    }
    let mut boot_time = Timeval { tv_sec: 0, tv_usec: 0 };
    let mut size = size_of::<Timeval>();
    // SAFETY: `boot_time` has room for `size` bytes, which is the size of the
    // `struct timeval` that `kern.boottime` returns.
    let result = unsafe {
        sysctlbyname(
            c"kern.boottime".as_ptr(),
            (&raw mut boot_time).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 || size != size_of::<Timeval>() || boot_time.tv_sec <= 0 {
        error!("Could not read the boot time: {}", io::Error::last_os_error());
        return None;
    }
    Some(format!("{}.{:06}", boot_time.tv_sec, boot_time.tv_usec))
}

unsafe extern "C" {
    fn sysctlbyname(
        name: *const c_char,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *mut c_void,
        newlen: usize,
    ) -> c_int;
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::{ContextsStore, Loaded, boot_id, empty_contexts_after};
    use crate::actor::app::WindowId;
    use crate::actor::parked_journal::FailingWrites;
    use crate::model::contexts::{ContextKey, Contexts, WindowDesc};
    use crate::sys::window_server::WindowServerId;

    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_790_000_000)
    }

    fn path(dir: &TempDir) -> PathBuf {
        dir.path().join("contexts.json")
    }

    fn file_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn comms() -> Contexts {
        let mut contexts = Contexts::new();
        let id = contexts.create("Comms").unwrap();
        contexts
            .add_window(
                id,
                &WindowDesc {
                    wid: WindowId::new(1, 1),
                    bundle_id: Some("net.whatsapp.WhatsApp".into()),
                    app_name: Some("WhatsApp".into()),
                    title: "WhatsApp".into(),
                    window_server_id: Some(WindowServerId::new(81234)),
                },
            )
            .unwrap();
        contexts.switch_to(ContextKey::Named(id)).unwrap();
        contexts
    }

    fn read(store: &ContextsStore) -> (Contexts, Option<String>) {
        match store.load(now()) {
            Loaded::Read { contexts, boot_id } => (contexts, boot_id),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn it_writes_the_documented_shape_with_the_boot_id() {
        let dir = TempDir::new().unwrap();
        let store = ContextsStore::new(path(&dir));
        store.save(&comms(), Some("1790000000.000001")).unwrap();

        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(path(&dir)).unwrap()).unwrap();
        assert_eq!(
            serde_json::json!({
                "version": 1,
                "next_id": 2,
                "use_seq": 1,
                "contexts": [
                    { "id": 1, "name": "Comms", "number": 1, "last_used": 1,
                      "members": [
                          { "bundle_id": "net.whatsapp.WhatsApp", "app_name": "WhatsApp",
                            "title": "WhatsApp", "window_server_id": 81234 }
                      ] }
                ],
                "pinned": [],
                "active": { "global": 1 },
                "boot_id": "1790000000.000001",
            }),
            written
        );
        assert_eq!(vec!["contexts.json"], file_names(dir.path()));
    }

    #[test]
    fn it_reads_back_what_it_wrote() {
        let dir = TempDir::new().unwrap();
        let store = ContextsStore::new(path(&dir));
        let contexts = comms();
        store.save(&contexts, Some("boot")).unwrap();

        let (read_back, boot) = read(&store);

        assert_eq!(Some("boot".to_string()), boot);
        assert_eq!(contexts.contexts().len(), read_back.contexts().len());
        assert_eq!(contexts.active(), read_back.active());
        assert_eq!(
            serde_json::to_value(&contexts).unwrap(),
            serde_json::to_value(&read_back).unwrap()
        );
    }

    #[test]
    fn a_file_without_a_boot_id_reads_as_from_an_unknown_boot() {
        let dir = TempDir::new().unwrap();
        let store = ContextsStore::new(path(&dir));
        store.save(&comms(), None).unwrap();
        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(path(&dir)).unwrap()).unwrap();
        assert!(written.get("boot_id").is_none());

        assert_eq!(None, read(&store).1);
    }

    #[test]
    fn a_missing_file_is_missing_and_is_not_created() {
        let dir = TempDir::new().unwrap();
        let store = ContextsStore::new(path(&dir));
        assert!(matches!(store.load(now()), Loaded::Missing));
        assert!(file_names(dir.path()).is_empty());
    }

    #[test]
    fn it_repairs_a_file_as_the_model_does() {
        let dir = TempDir::new().unwrap();
        fs::write(
            path(&dir),
            r#"{ "version": 1, "contexts": [
                { "id": 4, "name": "Everything", "number": 12 },
                { "id": 4, "name": "work" }
            ], "active": { "global": "nonsense" } }"#,
        )
        .unwrap();

        let (contexts, _) = read(&ContextsStore::new(path(&dir)));

        let names: Vec<_> = contexts.contexts().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(vec!["Everything 2", "work"], names);
        assert_ne!(contexts.contexts()[0].id, contexts.contexts()[1].id);
        assert_eq!(None, contexts.contexts()[0].number);
        assert_eq!(ContextKey::Everything, contexts.active());
    }

    /// Opens a file holding `contents`. The file must be moved aside
    /// unchanged, and a later save must write a new file.
    fn assert_moved_aside(contents: &[u8]) {
        let dir = TempDir::new().unwrap();
        fs::write(path(&dir), contents).unwrap();
        let store = ContextsStore::new(path(&dir));

        assert!(matches!(store.load(now()), Loaded::Unreadable));

        let aside = "contexts.unreadable-1790000000.json";
        assert_eq!(vec![aside], file_names(dir.path()));
        assert_eq!(contents, fs::read(dir.path().join(aside)).unwrap());
        store.save(&comms(), None).unwrap();
        assert_eq!(1, read(&store).0.contexts().len());
        assert_eq!(contents, fs::read(dir.path().join(aside)).unwrap());
    }

    #[test]
    fn an_unreadable_file_is_moved_aside_unchanged() {
        assert_moved_aside(b"");
        assert_moved_aside(b"{ not json");
        assert_moved_aside(br#"{ "version": 1, "contexts": [ { "id": 1, "name": "Co"#);
        assert_moved_aside(br#"{ "version": 2, "contexts": [] }"#);
        assert_moved_aside(br#"{ "contexts": [] }"#);
        assert_moved_aside(br#"{ "version": 1, "contexts": [ { "name": "No id" } ] }"#);
    }

    #[test]
    fn a_failed_save_leaves_the_old_file() {
        let dir = TempDir::new().unwrap();
        let store = ContextsStore::new(path(&dir));
        store.save(&Contexts::new(), None).unwrap();
        let before = fs::read(path(&dir)).unwrap();

        let failing = FailingWrites::start(dir.path());
        let result = store.save(&comms(), None);
        drop(failing);

        assert!(result.is_err());
        assert_eq!(before, fs::read(path(&dir)).unwrap());
    }

    #[test]
    fn an_in_memory_store_reads_nothing_and_writes_nothing() {
        let store = ContextsStore::in_memory();
        assert!(matches!(store.load(now()), Loaded::Missing));
        store.save(&comms(), Some("boot")).unwrap();
        assert!(matches!(store.load(now()), Loaded::Missing));
    }

    #[test]
    fn empty_contexts_take_ids_after_the_one_given() {
        let mut contexts = empty_contexts_after(None);
        assert_eq!(1, contexts.create("A").unwrap().get());
        let taken = contexts.create("B").unwrap();

        let mut after = empty_contexts_after(Some(taken));

        assert!(after.contexts().is_empty());
        assert_eq!(ContextKey::Everything, after.active());
        assert_eq!(taken.get() + 1, after.create("C").unwrap().get());
    }

    #[test]
    fn the_boot_id_stays_the_same_within_a_boot() {
        let first = boot_id();
        assert!(first.is_some());
        assert_eq!(first, boot_id());
    }
}
