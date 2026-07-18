use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, TryLockError,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tempfile::Builder as TempFileBuilder;

use crate::engines::{curated_whisper_models, resolve_curated_whisper_model, WhisperModelManifest};

const DOWNLOAD_PROGRESS_EVENT: &str = "models://download-progress";
const DOWNLOAD_BUFFER_BYTES: usize = 128 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(125);
const DOWNLOAD_GLOBAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelInstallationState {
    NotInstalled,
    NeedsVerification,
    Installed,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelSummary {
    pub manifest: WhisperModelManifest,
    pub state: ModelInstallationState,
    pub installed_bytes: u64,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelDownloadPhase {
    Downloading,
    Verifying,
    Installed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadProgress {
    pub model_id: String,
    pub phase: ModelDownloadPhase,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

impl ModelDownloadProgress {
    fn new(model: &WhisperModelManifest, phase: ModelDownloadPhase, downloaded_bytes: u64) -> Self {
        Self {
            model_id: model.id.clone(),
            phase,
            downloaded_bytes,
            total_bytes: model.download_bytes,
        }
    }
}

/// Curated local model storage with streamed integrity verification.
///
/// Only catalog identifiers are accepted, so a webview cannot choose an
/// arbitrary URL or filesystem target. Downloads are committed atomically in
/// the model directory after both byte length and SHA-256 match.
pub struct ModelStore {
    root: PathBuf,
    agent: ureq::Agent,
    operations: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
    verified: Mutex<HashMap<String, FileFingerprint>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    bytes: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed_seconds: i64,
    #[cfg(unix)]
    changed_nanoseconds: i64,
}

impl ModelStore {
    pub fn new(root: PathBuf) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(DOWNLOAD_GLOBAL_TIMEOUT))
            .timeout_connect(Some(DOWNLOAD_CONNECT_TIMEOUT))
            .timeout_recv_response(Some(DOWNLOAD_RESPONSE_TIMEOUT))
            .build()
            .into();
        Self {
            root,
            agent,
            operations: Mutex::new(HashMap::new()),
            downloads: Mutex::new(HashMap::new()),
            verified: Mutex::new(HashMap::new()),
        }
    }

    pub fn catalog(&self, active_model_id: &str) -> Vec<LocalModelSummary> {
        let active_model_id = resolve_curated_whisper_model(active_model_id)
            .map(|model| model.id.clone())
            .unwrap_or_else(|| active_model_id.to_owned());

        curated_whisper_models()
            .iter()
            .map(|model| self.summary(model, model.id == active_model_id))
            .collect()
    }

    pub fn verified_model_path(&self, model_id: &str) -> Result<PathBuf, String> {
        self.verified_model_path_with_cancellation(model_id, None)
    }

    pub fn verified_model_path_cancellable(
        &self,
        model_id: &str,
        cancellation: &AtomicBool,
    ) -> Result<PathBuf, String> {
        self.verified_model_path_with_cancellation(model_id, Some(cancellation))
    }

    fn verified_model_path_with_cancellation(
        &self,
        model_id: &str,
        cancellation: Option<&AtomicBool>,
    ) -> Result<PathBuf, String> {
        ensure_not_cancelled(cancellation)?;
        let model = resolve_model(model_id)?;
        let operation = self.operation_for(&model.id)?;
        let _operation = try_model_operation(&operation, &model.display_name)?;
        fs::create_dir_all(&self.root).map_err(|error| {
            format!(
                "could not create local model directory {}: {error}",
                self.root.display()
            )
        })?;

        let path = self.path_for(&model);
        if !path.exists() {
            return Err(format!(
                "{} is not installed. Download it from Engines first.",
                model.display_name
            ));
        }
        let fingerprint = file_fingerprint(&path)?;
        if self
            .verified
            .lock()
            .map_err(|_| "local model verification cache is unavailable".to_string())?
            .get(&model.id)
            .is_some_and(|verified| verified == &fingerprint)
            && receipt_matches(&self.receipt_path(&model), &model.sha256)
        {
            return Ok(path);
        }
        if let Err(error) = verify_file(&path, &model, cancellation) {
            ensure_not_cancelled(cancellation)?;
            self.quarantine_invalid(&model).map_err(|recovery_error| {
                format!("{error}; the invalid file could not be set aside: {recovery_error}")
            })?;
            return Err(format!(
                "{error}. The file was set aside; download the model again."
            ));
        }
        write_receipt(&self.receipt_path(&model), &model.sha256)?;
        self.remember_verified(&model.id, fingerprint)?;
        Ok(path)
    }

    pub fn install<F>(&self, model_id: &str, mut progress: F) -> Result<LocalModelSummary, String>
    where
        F: FnMut(ModelDownloadProgress),
    {
        let model = resolve_model(model_id)?;
        let operation = self.operation_for(&model.id)?;
        let _operation = try_model_operation(&operation, &model.display_name)?;
        let download = self.begin_download(&model.id)?;
        fs::create_dir_all(&self.root).map_err(|error| {
            format!(
                "could not create local model directory {}: {error}",
                self.root.display()
            )
        })?;

        let target = self.path_for(&model);
        let existing_verification =
            verify_file(&target, &model, Some(download.cancellation.as_ref()));
        ensure_not_cancelled(Some(download.cancellation.as_ref()))?;
        if existing_verification.is_ok() {
            write_receipt(&self.receipt_path(&model), &model.sha256)?;
            self.remember_verified(&model.id, file_fingerprint(&target)?)?;
            progress(ModelDownloadProgress::new(
                &model,
                ModelDownloadPhase::Installed,
                model.download_bytes,
            ));
            return Ok(self.summary(&model, false));
        }
        self.quarantine_invalid(&model)?;
        self.remove_invalid_artifacts(&model)?;
        ensure_not_cancelled(Some(download.cancellation.as_ref()))?;

        let mut response = self
            .agent
            .get(&model.source_url)
            .header("User-Agent", "Spick/0.1")
            .call()
            .map_err(|error| format!("could not download {}: {error}", model.display_name))?;

        if let Some(content_length) = response.body().content_length() {
            if content_length != model.download_bytes {
                return Err(format!(
                    "{} download reported {content_length} bytes; expected {}",
                    model.display_name, model.download_bytes
                ));
            }
        }

        let mut temporary = TempFileBuilder::new()
            .prefix(".spick-model-")
            .suffix(".part")
            .tempfile_in(&self.root)
            .map_err(|error| format!("could not create a temporary model file: {error}"))?;
        let mut last_progress = Instant::now() - PROGRESS_INTERVAL;
        let downloaded_bytes = copy_and_verify(
            response.body_mut().as_reader(),
            temporary.as_file_mut(),
            model.download_bytes,
            &model.sha256,
            Some(download.cancellation.as_ref()),
            |downloaded_bytes| {
                if last_progress.elapsed() >= PROGRESS_INTERVAL
                    || downloaded_bytes == model.download_bytes
                {
                    last_progress = Instant::now();
                    progress(ModelDownloadProgress::new(
                        &model,
                        ModelDownloadPhase::Downloading,
                        downloaded_bytes,
                    ));
                }
            },
        )?;
        ensure_not_cancelled(Some(download.cancellation.as_ref()))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| format!("could not finish the model download: {error}"))?;
        progress(ModelDownloadProgress::new(
            &model,
            ModelDownloadPhase::Verifying,
            downloaded_bytes,
        ));

        temporary.persist(&target).map_err(|error| {
            format!(
                "could not install {} at {}: {}",
                model.display_name,
                target.display(),
                error.error
            )
        })?;
        write_receipt(&self.receipt_path(&model), &model.sha256)?;
        self.remember_verified(&model.id, file_fingerprint(&target)?)?;
        self.remove_invalid_artifacts(&model)?;
        sync_parent_directory(&self.root);

        progress(ModelDownloadProgress::new(
            &model,
            ModelDownloadPhase::Installed,
            downloaded_bytes,
        ));
        Ok(self.summary(&model, false))
    }

    pub fn cancel_download(&self, model_id: &str) -> Result<bool, String> {
        let model = resolve_model(model_id)?;
        let downloads = self
            .downloads
            .lock()
            .map_err(|_| "local model download registry is unavailable".to_string())?;
        if let Some(cancellation) = downloads.get(&model.id) {
            cancellation.store(true, Ordering::Relaxed);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn remove(&self, model_id: &str) -> Result<(), String> {
        let model = resolve_model(model_id)?;
        let operation = self.operation_for(&model.id)?;
        let _operation = try_model_operation(&operation, &model.display_name)?;
        remove_if_present(&self.path_for(&model))?;
        remove_if_present(&self.receipt_path(&model))?;
        self.remove_invalid_artifacts(&model)?;
        self.verified
            .lock()
            .map_err(|_| "local model verification cache is unavailable".to_string())?
            .remove(&model.id);
        sync_parent_directory(&self.root);
        Ok(())
    }

    fn summary(&self, model: &WhisperModelManifest, active: bool) -> LocalModelSummary {
        let path = self.path_for(model);
        let target_bytes = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let invalid_bytes = self.invalid_artifact_bytes(model);
        let installed_bytes = if path.exists() {
            target_bytes
        } else {
            invalid_bytes
        };
        let state = if !path.exists() && invalid_bytes > 0 {
            ModelInstallationState::Invalid
        } else if target_bytes == 0 && !path.exists() {
            ModelInstallationState::NotInstalled
        } else if target_bytes != model.download_bytes {
            ModelInstallationState::Invalid
        } else if receipt_matches(&self.receipt_path(model), &model.sha256) {
            ModelInstallationState::Installed
        } else {
            ModelInstallationState::NeedsVerification
        };

        LocalModelSummary {
            manifest: model.clone(),
            state,
            installed_bytes,
            active,
        }
    }

    fn path_for(&self, model: &WhisperModelManifest) -> PathBuf {
        self.root.join(&model.file_name)
    }

    fn receipt_path(&self, model: &WhisperModelManifest) -> PathBuf {
        self.root.join(format!("{}.sha256", model.file_name))
    }

    fn remember_verified(
        &self,
        model_id: &str,
        fingerprint: FileFingerprint,
    ) -> Result<(), String> {
        self.verified
            .lock()
            .map_err(|_| "local model verification cache is unavailable".to_string())?
            .insert(model_id.to_owned(), fingerprint);
        Ok(())
    }

    fn operation_for(&self, model_id: &str) -> Result<Arc<Mutex<()>>, String> {
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| "local model operation registry is unavailable".to_string())?;
        Ok(Arc::clone(
            operations
                .entry(model_id.to_owned())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        ))
    }

    fn begin_download(&self, model_id: &str) -> Result<DownloadRegistration<'_>, String> {
        let cancellation = Arc::new(AtomicBool::new(false));
        let mut downloads = self
            .downloads
            .lock()
            .map_err(|_| "local model download registry is unavailable".to_string())?;
        if downloads.contains_key(model_id) {
            return Err("that model is already downloading".into());
        }
        downloads.insert(model_id.to_owned(), Arc::clone(&cancellation));
        Ok(DownloadRegistration {
            cancellation,
            downloads: &self.downloads,
            model_id: model_id.to_owned(),
        })
    }

    fn quarantine_invalid(&self, model: &WhisperModelManifest) -> Result<(), String> {
        remove_if_present(&self.receipt_path(model))?;
        self.verified
            .lock()
            .map_err(|_| "local model verification cache is unavailable".to_string())?
            .remove(&model.id);

        let target = self.path_for(model);
        if target.exists() {
            let quarantine = self
                .root
                .join(format!("{}.invalid-{}", model.file_name, now_ms()));
            fs::rename(&target, &quarantine).map_err(|error| {
                format!(
                    "could not preserve the invalid model at {}: {error}",
                    quarantine.display()
                )
            })?;
        }
        Ok(())
    }

    fn invalid_artifact_bytes(&self, model: &WhisperModelManifest) -> u64 {
        invalid_artifacts(&self.root, model)
            .filter_map(|path| fs::metadata(path).ok().map(|metadata| metadata.len()))
            .fold(0, u64::saturating_add)
    }

    fn remove_invalid_artifacts(&self, model: &WhisperModelManifest) -> Result<(), String> {
        for path in invalid_artifacts(&self.root, model) {
            remove_if_present(&path)?;
        }
        Ok(())
    }
}

struct DownloadRegistration<'a> {
    cancellation: Arc<AtomicBool>,
    downloads: &'a Mutex<HashMap<String, Arc<AtomicBool>>>,
    model_id: String,
}

impl Drop for DownloadRegistration<'_> {
    fn drop(&mut self) {
        if let Ok(mut downloads) = self.downloads.lock() {
            downloads.remove(&self.model_id);
        }
    }
}

pub const MODEL_DOWNLOAD_PROGRESS_EVENT: &str = DOWNLOAD_PROGRESS_EVENT;

fn resolve_model(model_id: &str) -> Result<std::sync::Arc<WhisperModelManifest>, String> {
    resolve_curated_whisper_model(model_id)
        .ok_or_else(|| format!("unknown local model: {model_id}"))
}

fn try_model_operation<'a>(
    operation: &'a Mutex<()>,
    display_name: &str,
) -> Result<MutexGuard<'a, ()>, String> {
    match operation.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => Err(format!(
            "{display_name} is already being downloaded, verified, or removed"
        )),
        Err(TryLockError::Poisoned(_)) => Err("local model store is unavailable".into()),
    }
}

fn invalid_artifacts<'a>(
    root: &'a Path,
    model: &'a WhisperModelManifest,
) -> impl Iterator<Item = PathBuf> + 'a {
    let prefix = format!("{}.invalid-", model.file_name);
    fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(move |entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .map(|entry| entry.path())
}

fn verify_file(
    path: &Path,
    model: &WhisperModelManifest,
    cancellation: Option<&AtomicBool>,
) -> Result<(), String> {
    ensure_not_cancelled(cancellation)?;
    let metadata = fs::metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!(
                "{} is not installed. Download it from Engines first.",
                model.display_name
            )
        } else {
            format!("could not inspect {}: {error}", model.display_name)
        }
    })?;
    if !metadata.is_file() {
        return Err(format!("{} is not a regular file", model.display_name));
    }
    if metadata.len() != model.download_bytes {
        return Err(format!(
            "{} has {} bytes; expected {}",
            model.display_name,
            metadata.len(),
            model.download_bytes
        ));
    }

    let file = File::open(path)
        .map_err(|error| format!("could not read {}: {error}", model.display_name))?;
    let digest = digest_reader(file, cancellation)?;
    if digest != model.sha256 {
        return Err(format!("{} failed its SHA-256 check", model.display_name));
    }
    Ok(())
}

fn file_fingerprint(path: &Path) -> Result<FileFingerprint, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    let fingerprint = FileFingerprint {
        bytes: metadata.len(),
        modified: metadata.modified().ok(),
        created: metadata.created().ok(),
        #[cfg(unix)]
        device: {
            use std::os::unix::fs::MetadataExt;
            metadata.dev()
        },
        #[cfg(unix)]
        inode: {
            use std::os::unix::fs::MetadataExt;
            metadata.ino()
        },
        #[cfg(unix)]
        changed_seconds: {
            use std::os::unix::fs::MetadataExt;
            metadata.ctime()
        },
        #[cfg(unix)]
        changed_nanoseconds: {
            use std::os::unix::fs::MetadataExt;
            metadata.ctime_nsec()
        },
    };
    Ok(fingerprint)
}

fn copy_and_verify<R, W, F>(
    mut reader: R,
    mut writer: W,
    expected_bytes: u64,
    expected_sha256: &str,
    cancellation: Option<&AtomicBool>,
    mut progress: F,
) -> Result<u64, String>
where
    R: Read,
    W: Write,
    F: FnMut(u64),
{
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; DOWNLOAD_BUFFER_BYTES];
    let mut downloaded_bytes = 0_u64;

    loop {
        ensure_not_cancelled(cancellation)?;
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("model download stopped early: {error}"))?;
        if read == 0 {
            break;
        }
        downloaded_bytes = downloaded_bytes
            .checked_add(read as u64)
            .ok_or_else(|| "model download is too large".to_string())?;
        if downloaded_bytes > expected_bytes {
            return Err(format!(
                "model download exceeded its declared {expected_bytes} bytes"
            ));
        }

        writer
            .write_all(&buffer[..read])
            .map_err(|error| format!("could not save the model download: {error}"))?;
        hasher.update(&buffer[..read]);
        progress(downloaded_bytes);
    }

    ensure_not_cancelled(cancellation)?;
    if downloaded_bytes != expected_bytes {
        return Err(format!(
            "model download contained {downloaded_bytes} bytes; expected {expected_bytes}"
        ));
    }
    let digest = hex_digest(hasher.finalize().as_slice());
    if digest != expected_sha256 {
        return Err("model download failed its SHA-256 check".into());
    }
    Ok(downloaded_bytes)
}

fn ensure_not_cancelled(cancellation: Option<&AtomicBool>) -> Result<(), String> {
    if cancellation.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        Err("model operation was cancelled".into())
    } else {
        Ok(())
    }
}

fn digest_reader<R: Read>(
    mut reader: R,
    cancellation: Option<&AtomicBool>,
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; DOWNLOAD_BUFFER_BYTES];
    loop {
        ensure_not_cancelled(cancellation)?;
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("could not verify the local model: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    ensure_not_cancelled(cancellation)?;
    Ok(hex_digest(hasher.finalize().as_slice()))
}

fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut value, "{byte:02x}").expect("writing to a String cannot fail");
    }
    value
}

fn write_receipt(path: &Path, sha256: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "model receipt path has no parent directory".to_string())?;
    let mut temporary = TempFileBuilder::new()
        .prefix(".spick-model-receipt-")
        .tempfile_in(parent)
        .map_err(|error| format!("could not create a model receipt: {error}"))?;
    writeln!(temporary, "{sha256}")
        .map_err(|error| format!("could not write the model receipt: {error}"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("could not finish the model receipt: {error}"))?;
    temporary
        .persist(path)
        .map_err(|error| format!("could not save the model receipt: {}", error.error))?;
    Ok(())
}

fn receipt_matches(path: &Path, sha256: &str) -> bool {
    fs::read_to_string(path)
        .map(|receipt| receipt.trim() == sha256)
        .unwrap_or(false)
}

fn remove_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not remove {}: {error}", path.display())),
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) {
    if let Ok(directory) = File::open(path) {
        let _ = directory.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) {}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn streamed_download_requires_exact_size_and_digest() {
        let mut output = Vec::new();
        let mut updates = Vec::new();
        assert_eq!(
            copy_and_verify(
                Cursor::new(b"abc"),
                &mut output,
                3,
                ABC_SHA256,
                None,
                |bytes| updates.push(bytes),
            )
            .unwrap(),
            3
        );
        assert_eq!(output, b"abc");
        assert_eq!(updates, vec![3]);

        assert!(
            copy_and_verify(Cursor::new(b"ab"), Vec::new(), 3, ABC_SHA256, None, |_| {}).is_err()
        );
        assert!(copy_and_verify(
            Cursor::new(b"abcd"),
            Vec::new(),
            3,
            ABC_SHA256,
            None,
            |_| {}
        )
        .is_err());
        assert!(copy_and_verify(
            Cursor::new(b"abc"),
            Vec::new(),
            3,
            &"0".repeat(64),
            None,
            |_| {}
        )
        .is_err());

        let cancelled = AtomicBool::new(true);
        assert!(copy_and_verify(
            Cursor::new(b"abc"),
            Vec::new(),
            3,
            ABC_SHA256,
            Some(&cancelled),
            |_| {}
        )
        .unwrap_err()
        .contains("cancelled"));
    }

    #[test]
    fn local_catalog_starts_without_claiming_models_are_installed() {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelStore::new(directory.path().to_path_buf());
        let catalog = store.catalog("whisper-small-multilingual-q5-1");

        assert_eq!(catalog.len(), curated_whisper_models().len());
        assert!(catalog
            .iter()
            .all(|model| model.state == ModelInstallationState::NotInstalled));
        assert_eq!(catalog.iter().filter(|model| model.active).count(), 1);
    }

    #[test]
    fn unknown_identifiers_cannot_escape_the_model_directory() {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelStore::new(directory.path().to_path_buf());
        assert!(store.verified_model_path("../../outside").is_err());
        assert!(store.remove("https://example.com/model.bin").is_err());
    }

    #[test]
    fn one_model_operation_does_not_block_another_model() {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelStore::new(directory.path().to_path_buf());
        let first = store.operation_for("first").unwrap();
        let same = store.operation_for("first").unwrap();
        let second = store.operation_for("second").unwrap();
        assert!(Arc::ptr_eq(&first, &same));
        assert!(!Arc::ptr_eq(&first, &second));

        let _first_guard = first.lock().unwrap();
        assert!(try_model_operation(&first, "First").is_err());
        assert!(try_model_operation(&second, "Second").is_ok());
    }

    #[test]
    fn quarantined_models_are_visible_and_removed_with_the_model() {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelStore::new(directory.path().to_path_buf());
        let model = &curated_whisper_models()[0];
        let quarantine = directory
            .path()
            .join(format!("{}.invalid-1", model.file_name));
        fs::write(&quarantine, b"bad model").unwrap();

        let summary = store
            .catalog(&model.id)
            .into_iter()
            .find(|summary| summary.manifest.id == model.id)
            .unwrap();
        assert_eq!(summary.state, ModelInstallationState::Invalid);
        assert_eq!(summary.installed_bytes, 9);

        store.remove(&model.id).unwrap();
        assert!(!quarantine.exists());
    }

    #[test]
    fn active_downloads_can_be_cancelled_and_are_unregistered_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelStore::new(directory.path().to_path_buf());
        let model = &curated_whisper_models()[0];
        let registration = store.begin_download(&model.id).unwrap();

        assert!(store.cancel_download(&model.id).unwrap());
        assert!(registration.cancellation.load(Ordering::Relaxed));
        drop(registration);
        assert!(!store.cancel_download(&model.id).unwrap());
    }
}
