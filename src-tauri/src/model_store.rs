use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, TryLockError,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::Builder as TempFileBuilder;

use crate::engines::{
    curated_whisper_models, inspect_whisper_model, resolve_curated_whisper_model, ModelLanguageSet,
    WhisperModelInspection, WhisperModelManifest, WhisperModelOrigin, WhisperQuantization,
};

const DOWNLOAD_PROGRESS_EVENT: &str = "models://download-progress";
const DOWNLOAD_BUFFER_BYTES: usize = 128 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(125);
const DOWNLOAD_GLOBAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const IMPORTED_MODELS_SCHEMA_VERSION: u32 = 1;
const IMPORTED_MODELS_FILE: &str = "imported-models.json";
const MAX_IMPORTED_MODELS: usize = 256;
const MAX_IMPORTED_REGISTRY_BYTES: u64 = 2 * 1024 * 1024;
const MAX_IMPORTED_MODEL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MODEL_OPERATION_POLL_INTERVAL: Duration = Duration::from_millis(10);

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

/// Local model storage with streamed integrity verification.
///
/// Curated downloads accept only catalog identifiers and pinned HTTPS sources.
/// Imports accept only a path returned by the native file picker, copy the
/// selected trusted GGML file into app-local storage, and never expose or retain
/// that source path in the webview or registry.
pub struct ModelStore {
    root: PathBuf,
    agent: ureq::Agent,
    operations: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
    verified: Mutex<HashMap<String, FileFingerprint>>,
    imported: Mutex<HashMap<String, Arc<WhisperModelManifest>>>,
    imported_registry_error: Option<String>,
    import_gate: Mutex<()>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportedModelsRegistry {
    schema_version: u32,
    models: Vec<WhisperModelManifest>,
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

#[derive(Clone, Copy)]
struct ImportArtifacts<'a> {
    receipt: &'a Path,
    target: &'a Path,
    wrote_receipt: bool,
    installed_fresh_copy: bool,
}

impl ImportArtifacts<'_> {
    fn rollback_after(self, primary_error: String) -> String {
        let mut cleanup_errors = Vec::new();
        if self.wrote_receipt {
            if let Err(error) = remove_if_present(self.receipt) {
                cleanup_errors.push(error);
            }
        }
        if self.installed_fresh_copy {
            if let Err(error) = remove_if_present(self.target) {
                cleanup_errors.push(error);
            }
        }
        if cleanup_errors.is_empty() {
            primary_error
        } else {
            format!(
                "{primary_error}; Spick could not fully clean up the failed import: {}",
                cleanup_errors.join("; ")
            )
        }
    }
}

impl ModelStore {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(DOWNLOAD_GLOBAL_TIMEOUT))
            .timeout_connect(Some(DOWNLOAD_CONNECT_TIMEOUT))
            .timeout_recv_response(Some(DOWNLOAD_RESPONSE_TIMEOUT))
            .build()
            .into();
        let (imported, imported_registry_error) = match load_imported_registry(&root) {
            Ok(imported) => (imported, None),
            Err(error) => {
                eprintln!("imported local models are unavailable: {error}");
                (HashMap::new(), Some(error))
            }
        };
        Ok(Self {
            root,
            agent,
            operations: Mutex::new(HashMap::new()),
            downloads: Mutex::new(HashMap::new()),
            verified: Mutex::new(HashMap::new()),
            imported: Mutex::new(imported),
            imported_registry_error,
            import_gate: Mutex::new(()),
        })
    }

    pub fn catalog(&self, active_model_id: &str) -> Vec<LocalModelSummary> {
        let active_model_id = self
            .resolve(active_model_id)
            .map(|model| model.id.clone())
            .unwrap_or_else(|| active_model_id.to_owned());

        let mut models = curated_whisper_models()
            .iter()
            .map(|model| self.summary(model, model.id == active_model_id))
            .collect::<Vec<_>>();
        let mut imported = self
            .imported
            .lock()
            .map(|models| models.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        imported.sort_by(|left, right| {
            left.display_name
                .to_lowercase()
                .cmp(&right.display_name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        models.extend(
            imported
                .iter()
                .map(|model| self.summary(model, model.id == active_model_id)),
        );
        models
    }

    pub fn resolve(&self, model_id: &str) -> Option<Arc<WhisperModelManifest>> {
        resolve_curated_whisper_model(model_id).or_else(|| {
            self.imported
                .lock()
                .ok()
                .and_then(|models| models.get(model_id).cloned())
        })
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
        let model = self.resolve_model(model_id)?;
        let operation = self.operation_for(&model.id)?;
        let _operation = wait_for_model_operation(&operation, &model.display_name, cancellation)?;
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
        let model = resolve_download_model(model_id)?;
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

        let source_url = model
            .source_url
            .as_deref()
            .ok_or_else(|| "imported models cannot be downloaded".to_string())?;
        let mut response = self
            .agent
            .get(source_url)
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

    /// Copies a user-selected whisper.cpp model into app-local storage.
    ///
    /// The source path never leaves the native process and is not retained.
    /// IDs and destination filenames are derived only from the copied content,
    /// so user-controlled path components cannot escape the model directory.
    pub fn import_from_path(&self, source: &Path) -> Result<LocalModelSummary, String> {
        if let Some(error) = self.imported_registry_error.as_deref() {
            return Err(format!(
                "imported model metadata needs attention before another model can be added: {error}"
            ));
        }
        let _import = self
            .import_gate
            .lock()
            .map_err(|_| "local model import is unavailable".to_string())?;
        validate_import_source(source)?;
        fs::create_dir_all(&self.root).map_err(|error| {
            format!(
                "could not create local model directory {}: {error}",
                self.root.display()
            )
        })?;

        let mut temporary = TempFileBuilder::new()
            .prefix(".spick-import-")
            .suffix(".bin")
            .tempfile_in(&self.root)
            .map_err(|error| format!("could not create a temporary model file: {error}"))?;
        let (model_bytes, sha256) = copy_import_source(source, temporary.as_file_mut())?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| format!("could not finish the imported model copy: {error}"))?;
        let model_id = format!("whisper-imported-{sha256}");
        let (existing, imported_count) = {
            let imported = self
                .imported
                .lock()
                .map_err(|_| "imported model registry is unavailable".to_string())?;
            (imported.get(&model_id).cloned(), imported.len())
        };
        if existing.is_none() && imported_count >= MAX_IMPORTED_MODELS {
            return Err("remove an imported model before adding another one".into());
        }
        validate_ggml_header(temporary.as_file_mut())?;
        let inspection =
            inspect_whisper_model(temporary.path()).map_err(|error| error.to_string())?;

        let language_marker = if inspection.languages == ModelLanguageSet::EnglishOnly {
            ".en"
        } else {
            ""
        };
        let file_name = format!("{model_id}{language_marker}.bin");
        let display_name = imported_display_name(&inspection, &sha256);
        let inspected_model = Arc::new(WhisperModelManifest {
            id: model_id,
            // Filenames are user-controlled and can be misleading. Derive the
            // persisted label only from inspected metadata and content hash.
            display_name,
            file_name,
            family: inspection.family,
            languages: inspection.languages,
            quantization: inspection.quantization,
            download_bytes: model_bytes,
            sha256,
            origin: WhisperModelOrigin::Imported,
            source_url: None,
            license: None,
        });
        inspected_model
            .validate()
            .map_err(|reason| format!("the imported model metadata is invalid: {reason}"))?;
        let registry_needs_update = existing.as_deref() != Some(inspected_model.as_ref());
        let model = inspected_model;

        let operation = self.operation_for(&model.id)?;
        let _operation = try_model_operation(&operation, &model.display_name)?;
        let target = self.path_for(&model);
        let installed_fresh_copy = if verify_file(&target, &model, None).is_ok() {
            false
        } else {
            self.quarantine_invalid(&model)?;
            self.remove_invalid_artifacts(&model)?;
            temporary
                .persist(&target)
                .map_err(|error| format!("could not save the imported model: {}", error.error))?;
            true
        };

        let receipt = self.receipt_path(&model);
        let wrote_receipt = !receipt_matches(&receipt, &model.sha256);
        if wrote_receipt {
            if let Err(error) = write_receipt(&receipt, &model.sha256) {
                return Err(ImportArtifacts {
                    receipt: &receipt,
                    target: &target,
                    wrote_receipt: false,
                    installed_fresh_copy,
                }
                .rollback_after(error));
            }
        }
        let artifacts = ImportArtifacts {
            receipt: &receipt,
            target: &target,
            wrote_receipt,
            installed_fresh_copy,
        };
        let fingerprint = match file_fingerprint(&target) {
            Ok(fingerprint) => fingerprint,
            Err(error) => return Err(artifacts.rollback_after(error)),
        };
        self.finish_imported_model(&model, fingerprint, registry_needs_update, artifacts)?;

        sync_parent_directory(&self.root);
        Ok(self.summary(&model, false))
    }

    fn finish_imported_model(
        &self,
        model: &Arc<WhisperModelManifest>,
        fingerprint: FileFingerprint,
        registry_needs_update: bool,
        artifacts: ImportArtifacts<'_>,
    ) -> Result<(), String> {
        if let Err(error) = self.commit_imported_model(model, fingerprint, registry_needs_update) {
            return Err(artifacts.rollback_after(error));
        }
        Ok(())
    }

    fn commit_imported_model(
        &self,
        model: &Arc<WhisperModelManifest>,
        fingerprint: FileFingerprint,
        registry_needs_update: bool,
    ) -> Result<(), String> {
        if !registry_needs_update {
            return self.remember_verified(&model.id, fingerprint);
        }

        // Acquire every fallible in-memory dependency before the durable
        // registry write. No other path holds these two locks in reverse.
        let mut imported = self
            .imported
            .lock()
            .map_err(|_| "imported model registry is unavailable".to_string())?;
        let mut verified = self
            .verified
            .lock()
            .map_err(|_| "local model verification cache is unavailable".to_string())?;
        let mut next = imported.clone();
        next.insert(model.id.clone(), Arc::clone(model));

        // This durable registry write is the logical commit. Everything after
        // it is deliberately infallible.
        persist_imported_registry(&self.root, &next)?;
        *imported = next;
        verified.insert(model.id.clone(), fingerprint);
        Ok(())
    }

    pub fn cancel_download(&self, model_id: &str) -> Result<bool, String> {
        let model = resolve_download_model(model_id)?;
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
        let model = self.resolve_model(model_id)?;
        // Imported registry mutations share the same lock order as import:
        // registry gate -> per-model operation -> registry map.
        let _import_registry_operation = if model.origin == WhisperModelOrigin::Imported {
            Some(
                self.import_gate
                    .lock()
                    .map_err(|_| "local model import registry is unavailable".to_string())?,
            )
        } else {
            None
        };
        let operation = self.operation_for(&model.id)?;
        let _operation = try_model_operation(&operation, &model.display_name)?;
        if model.origin == WhisperModelOrigin::Imported {
            let mut imported = self
                .imported
                .lock()
                .map_err(|_| "imported model registry is unavailable".to_string())?;
            let mut next = imported.clone();
            next.remove(&model.id);
            persist_imported_registry(&self.root, &next)?;
            *imported = next;
        }

        let removal = (|| {
            remove_if_present(&self.path_for(&model))?;
            remove_if_present(&self.receipt_path(&model))?;
            self.remove_invalid_artifacts(&model)?;
            self.verified
                .lock()
                .map_err(|_| "local model verification cache is unavailable".to_string())?
                .remove(&model.id);
            Ok::<(), String>(())
        })();
        if let Err(error) = removal {
            // Registry removal is the logical commit for imported models.
            // Never resurrect metadata after a partial physical deletion;
            // that would expose an impossible downloadable state in the UI.
            if model.origin == WhisperModelOrigin::Imported {
                sync_parent_directory(&self.root);
                return Err(format!(
                    "{} was removed from Spick, but some local files remain: {error}",
                    model.display_name
                ));
            }
            return Err(error);
        }
        sync_parent_directory(&self.root);
        Ok(())
    }

    fn resolve_model(&self, model_id: &str) -> Result<Arc<WhisperModelManifest>, String> {
        self.resolve(model_id)
            .ok_or_else(|| format!("unknown local model: {model_id}"))
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

fn resolve_download_model(model_id: &str) -> Result<Arc<WhisperModelManifest>, String> {
    resolve_curated_whisper_model(model_id)
        .ok_or_else(|| format!("unknown downloadable model: {model_id}"))
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

fn wait_for_model_operation<'a>(
    operation: &'a Mutex<()>,
    display_name: &str,
    cancellation: Option<&AtomicBool>,
) -> Result<MutexGuard<'a, ()>, String> {
    loop {
        ensure_not_cancelled(cancellation)?;
        match operation.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::WouldBlock) => std::thread::sleep(MODEL_OPERATION_POLL_INTERVAL),
            Err(TryLockError::Poisoned(_)) => {
                return Err(format!(
                "{display_name} could not be verified because the local model store is unavailable"
            ))
            }
        }
    }
}

fn load_imported_registry(
    root: &Path,
) -> Result<HashMap<String, Arc<WhisperModelManifest>>, String> {
    let path = root.join(IMPORTED_MODELS_FILE);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => {
            return Err(format!(
                "could not read the imported model registry: {error}"
            ))
        }
    };
    let metadata = file
        .metadata()
        .map_err(|error| format!("could not inspect the imported model registry: {error}"))?;
    if !metadata.is_file() {
        return Err("the imported model registry is not a regular file".into());
    }
    if metadata.len() > MAX_IMPORTED_REGISTRY_BYTES {
        return Err("the imported model registry is too large".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_IMPORTED_REGISTRY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read the imported model registry: {error}"))?;
    if bytes.len() as u64 > MAX_IMPORTED_REGISTRY_BYTES {
        return Err("the imported model registry is too large".into());
    }
    let registry: ImportedModelsRegistry = serde_json::from_slice(&bytes)
        .map_err(|error| format!("the imported model registry is invalid JSON: {error}"))?;
    if registry.schema_version != IMPORTED_MODELS_SCHEMA_VERSION {
        return Err(format!(
            "unsupported imported model registry schema: {}",
            registry.schema_version
        ));
    }
    if registry.models.len() > MAX_IMPORTED_MODELS {
        return Err("the imported model registry contains too many entries".into());
    }

    let mut models = HashMap::with_capacity(registry.models.len());
    for model in registry.models {
        validate_imported_manifest(&model)?;
        let id = model.id.clone();
        if models.insert(id.clone(), Arc::new(model)).is_some() {
            return Err(format!("the imported model registry repeats {id}"));
        }
    }
    Ok(models)
}

fn persist_imported_registry(
    root: &Path,
    models: &HashMap<String, Arc<WhisperModelManifest>>,
) -> Result<(), String> {
    if models.len() > MAX_IMPORTED_MODELS {
        return Err("the imported model registry contains too many entries".into());
    }
    fs::create_dir_all(root)
        .map_err(|error| format!("could not create the local model directory: {error}"))?;
    let mut manifests = models
        .values()
        .map(|model| model.as_ref().clone())
        .collect::<Vec<_>>();
    manifests.sort_by(|left, right| left.id.cmp(&right.id));
    for model in &manifests {
        validate_imported_manifest(model)?;
    }
    let registry = ImportedModelsRegistry {
        schema_version: IMPORTED_MODELS_SCHEMA_VERSION,
        models: manifests,
    };
    let mut bytes = serde_json::to_vec_pretty(&registry)
        .map_err(|error| format!("could not serialize imported model metadata: {error}"))?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_IMPORTED_REGISTRY_BYTES {
        return Err("the imported model registry is too large".into());
    }
    let mut temporary = TempFileBuilder::new()
        .prefix(".spick-imported-models-")
        .tempfile_in(root)
        .map_err(|error| format!("could not create imported model metadata: {error}"))?;
    temporary
        .write_all(&bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| format!("could not write imported model metadata: {error}"))?;
    temporary
        .persist(root.join(IMPORTED_MODELS_FILE))
        .map_err(|error| format!("could not save imported model metadata: {}", error.error))?;
    sync_parent_directory(root);
    Ok(())
}

fn validate_imported_manifest(model: &WhisperModelManifest) -> Result<(), String> {
    model
        .validate()
        .map_err(|reason| format!("invalid imported model {}: {reason}", model.id))?;
    if model.origin != WhisperModelOrigin::Imported {
        return Err(format!(
            "the imported model registry contains a non-imported model: {}",
            model.id
        ));
    }
    let expected_id = format!("whisper-imported-{}", model.sha256);
    if model.id != expected_id {
        return Err(format!(
            "imported model identity does not match its digest: {}",
            model.id
        ));
    }
    let language_marker = if model.languages == ModelLanguageSet::EnglishOnly {
        ".en"
    } else {
        ""
    };
    if model.file_name != format!("{}{language_marker}.bin", model.id) {
        return Err(format!(
            "imported model filename does not match its identity: {}",
            model.id
        ));
    }
    if !(48..=MAX_IMPORTED_MODEL_BYTES).contains(&model.download_bytes) {
        return Err(format!(
            "imported model size is outside the allowed range: {}",
            model.id
        ));
    }
    if !matches!(
        model.family,
        crate::engines::WhisperModelFamily::Tiny
            | crate::engines::WhisperModelFamily::Base
            | crate::engines::WhisperModelFamily::Small
            | crate::engines::WhisperModelFamily::Medium
            | crate::engines::WhisperModelFamily::Large
            | crate::engines::WhisperModelFamily::LargeV3
    ) {
        return Err(format!(
            "imported model family was not derived by this runtime: {}",
            model.id
        ));
    }
    if let WhisperQuantization::Other(value) = &model.quantization {
        let Some(raw_ftype) = value.strip_prefix("ftype-") else {
            return Err(format!(
                "imported model quantization is invalid: {}",
                model.id
            ));
        };
        if !raw_ftype
            .parse::<i32>()
            .is_ok_and(|value| (0..=100_000).contains(&value))
        {
            return Err(format!(
                "imported model quantization is invalid: {}",
                model.id
            ));
        }
    }
    let expected_display_name = imported_display_name(
        &WhisperModelInspection {
            family: model.family,
            languages: model.languages,
            quantization: model.quantization.clone(),
        },
        &model.sha256,
    );
    if model.display_name != expected_display_name {
        return Err(format!(
            "imported model display metadata does not match its inspected identity: {}",
            model.id
        ));
    }
    Ok(())
}

fn validate_import_source(source: &Path) -> Result<(), String> {
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if extension.as_deref() == Some("gguf") {
        return Err(
            "GGUF is not a whisper.cpp speech-model format in this build. Choose a whisper.cpp GGML .bin file."
                .into(),
        );
    }
    if extension.as_deref() != Some("bin") {
        return Err("choose a whisper.cpp GGML model with the .bin extension".into());
    }
    Ok(())
}

fn imported_display_name(inspection: &WhisperModelInspection, sha256: &str) -> String {
    let family = match inspection.family {
        crate::engines::WhisperModelFamily::Tiny => "Tiny",
        crate::engines::WhisperModelFamily::Base => "Base",
        crate::engines::WhisperModelFamily::Small => "Small",
        crate::engines::WhisperModelFamily::Medium => "Medium",
        crate::engines::WhisperModelFamily::LargeV1 => "Large v1",
        crate::engines::WhisperModelFamily::LargeV2 => "Large v2",
        crate::engines::WhisperModelFamily::Large => "Large",
        crate::engines::WhisperModelFamily::LargeV3 => "Large v3",
        crate::engines::WhisperModelFamily::LargeV3Turbo => "Large v3 Turbo",
    };
    let quantization = match &inspection.quantization {
        WhisperQuantization::F16 => "F16".to_string(),
        WhisperQuantization::F32 => "F32".to_string(),
        WhisperQuantization::Q4_0 => "Q4_0".to_string(),
        WhisperQuantization::Q4_1 => "Q4_1".to_string(),
        WhisperQuantization::Q5_0 => "Q5_0".to_string(),
        WhisperQuantization::Q5_1 => "Q5_1".to_string(),
        WhisperQuantization::Q8_0 => "Q8_0".to_string(),
        WhisperQuantization::Other(value) => value.clone(),
    };
    let digest = sha256.chars().take(8).collect::<String>();
    format!("Imported {family} {quantization} · {digest}")
}

fn copy_import_source(source: &Path, output: &mut File) -> Result<(u64, String), String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut input = options.open(source).map_err(|error| {
        #[cfg(unix)]
        if error.raw_os_error() == Some(libc::ELOOP) {
            return "choose the model file itself instead of a symbolic link".to_string();
        }
        format!("could not read the selected model: {error}")
    })?;
    let metadata = input
        .metadata()
        .map_err(|error| format!("could not inspect the selected model: {error}"))?;
    if !metadata.is_file() {
        return Err("the selected model is not a regular file".into());
    }
    let expected_bytes = metadata.len();
    if expected_bytes < 48 {
        return Err("the selected file is too small to be a whisper.cpp model".into());
    }
    if expected_bytes > MAX_IMPORTED_MODEL_BYTES {
        return Err("the selected model is larger than Spick's 8 GiB import limit".into());
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; DOWNLOAD_BUFFER_BYTES];
    let mut copied = 0_u64;
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("could not read the selected model: {error}"))?;
        if read == 0 {
            break;
        }
        copied = copied
            .checked_add(read as u64)
            .ok_or_else(|| "the selected model is too large".to_string())?;
        if copied > MAX_IMPORTED_MODEL_BYTES {
            return Err("the selected model exceeded Spick's 8 GiB import limit".into());
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| format!("could not copy the selected model: {error}"))?;
        hasher.update(&buffer[..read]);
    }
    if copied != expected_bytes {
        return Err("the selected model changed while Spick was importing it".into());
    }
    Ok((copied, hex_digest(hasher.finalize().as_slice())))
}

fn validate_ggml_header(file: &mut File) -> Result<(), String> {
    let mut header = [0_u8; 48];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut header))
        .map_err(|error| format!("could not inspect the copied model: {error}"))?;
    if &header[..4] == b"GGUF" {
        return Err(
            "GGUF is not a whisper.cpp speech-model format in this build. Choose a whisper.cpp GGML .bin file."
                .into(),
        );
    }
    if &header[..4] != b"lmgg" {
        return Err("the selected file is not a whisper.cpp GGML model".into());
    }
    let values = header[4..]
        .chunks_exact(4)
        .map(|bytes| i32::from_le_bytes(bytes.try_into().expect("four-byte header field")))
        .collect::<Vec<_>>();
    let [n_vocab, n_audio_ctx, n_audio_state, n_audio_head, n_audio_layer, n_text_ctx, n_text_state, n_text_head, n_text_layer, n_mels, ftype] =
        values.as_slice()
    else {
        return Err("the selected model has an incomplete GGML header".into());
    };
    let bounded = (1..=1_000_000).contains(n_vocab)
        && (1..=32_768).contains(n_audio_ctx)
        && (1..=16_384).contains(n_audio_state)
        && (1..=1_024).contains(n_audio_head)
        && matches!(n_audio_layer, 4 | 6 | 12 | 24 | 32)
        && (1..=32_768).contains(n_text_ctx)
        && (1..=16_384).contains(n_text_state)
        && (1..=1_024).contains(n_text_head)
        && (1..=1_024).contains(n_text_layer)
        && (1..=1_024).contains(n_mels)
        && (0..=100_000).contains(ftype)
        && n_audio_state == n_text_state;
    if !bounded {
        return Err("the selected GGML file has unsupported Whisper dimensions".into());
    }
    Ok(())
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
    use std::{io::Cursor, sync::mpsc, thread};

    use super::*;

    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn imported_manifest_for_digest(sha256: String) -> Arc<WhisperModelManifest> {
        let inspection = WhisperModelInspection {
            family: crate::engines::WhisperModelFamily::Small,
            languages: ModelLanguageSet::Multilingual,
            quantization: crate::engines::WhisperQuantization::Q5_1,
        };
        Arc::new(WhisperModelManifest {
            id: format!("whisper-imported-{sha256}"),
            display_name: imported_display_name(&inspection, &sha256),
            file_name: format!("whisper-imported-{sha256}.bin"),
            family: inspection.family,
            languages: inspection.languages,
            quantization: inspection.quantization,
            download_bytes: 123,
            sha256,
            origin: WhisperModelOrigin::Imported,
            source_url: None,
            license: None,
        })
    }

    fn imported_manifest() -> Arc<WhisperModelManifest> {
        imported_manifest_for_digest("a".repeat(64))
    }

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
        let store = ModelStore::new(directory.path().to_path_buf()).unwrap();
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
        let store = ModelStore::new(directory.path().to_path_buf()).unwrap();
        assert!(store.verified_model_path("../../outside").is_err());
        assert!(store.remove("https://example.com/model.bin").is_err());
    }

    #[test]
    fn one_model_operation_does_not_block_another_model() {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelStore::new(directory.path().to_path_buf()).unwrap();
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
    fn verification_waits_for_an_existing_operation_and_honors_cancellation() {
        let operation = Arc::new(Mutex::new(()));
        let held = operation.lock().unwrap();
        let (finished_tx, finished_rx) = mpsc::channel();
        let waiting = Arc::clone(&operation);
        let worker = thread::spawn(move || {
            let result = wait_for_model_operation(&waiting, "Model", None).map(drop);
            finished_tx.send(result).unwrap();
        });
        assert!(matches!(
            finished_rx.recv_timeout(Duration::from_millis(40)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(held);
        finished_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        worker.join().unwrap();

        let held = operation.lock().unwrap();
        let cancellation = Arc::new(AtomicBool::new(false));
        let waiting = Arc::clone(&operation);
        let worker_cancellation = Arc::clone(&cancellation);
        let worker = thread::spawn(move || {
            wait_for_model_operation(&waiting, "Model", Some(&worker_cancellation)).map(drop)
        });
        cancellation.store(true, Ordering::Relaxed);
        assert!(worker.join().unwrap().unwrap_err().contains("cancelled"));
        drop(held);
    }

    #[test]
    fn imported_registry_round_trips_without_persisting_a_source_path() {
        let directory = tempfile::tempdir().unwrap();
        let model = imported_manifest();
        let models = HashMap::from([(model.id.clone(), Arc::clone(&model))]);
        persist_imported_registry(directory.path(), &models).unwrap();

        let raw = fs::read_to_string(directory.path().join(IMPORTED_MODELS_FILE)).unwrap();
        assert!(!raw.contains("/Users/"));
        assert!(!raw.contains("sourceUrl"));
        let store = ModelStore::new(directory.path().to_path_buf()).unwrap();
        assert_eq!(store.resolve(&model.id).as_deref(), Some(model.as_ref()));
        assert_eq!(
            store.catalog(&model.id).last().unwrap().manifest.origin,
            WhisperModelOrigin::Imported
        );
    }

    #[test]
    fn imported_registry_refuses_more_than_the_supported_entry_limit() {
        let directory = tempfile::tempdir().unwrap();
        let models = (0..=MAX_IMPORTED_MODELS)
            .map(|index| {
                let model = imported_manifest_for_digest(format!("{index:064x}"));
                (model.id.clone(), model)
            })
            .collect::<HashMap<_, _>>();

        assert!(persist_imported_registry(directory.path(), &models)
            .unwrap_err()
            .contains("too many"));
        assert!(!directory.path().join(IMPORTED_MODELS_FILE).exists());
    }

    #[test]
    fn imported_registry_rejects_spoofed_display_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let mut model = imported_manifest().as_ref().clone();
        model.display_name = "Definitely the official model".into();
        let models = HashMap::from([(model.id.clone(), Arc::new(model))]);

        assert!(persist_imported_registry(directory.path(), &models)
            .unwrap_err()
            .contains("display metadata"));
        assert!(!directory.path().join(IMPORTED_MODELS_FILE).exists());
    }

    #[test]
    fn import_cache_failure_cannot_publish_a_registry_entry() {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelStore::new(directory.path().to_path_buf()).unwrap();
        let model = imported_manifest();
        let fingerprint_source = directory.path().join("fingerprint.bin");
        fs::write(&fingerprint_source, b"fingerprint").unwrap();
        let fingerprint = file_fingerprint(&fingerprint_source).unwrap();

        assert!(std::panic::catch_unwind(|| {
            let _verified = store.verified.lock().unwrap();
            panic!("poison verification cache");
        })
        .is_err());
        let error = store
            .commit_imported_model(&model, fingerprint, true)
            .unwrap_err();

        assert!(error.contains("verification cache"));
        assert!(store.resolve(&model.id).is_none());
        assert!(!directory.path().join(IMPORTED_MODELS_FILE).exists());
    }

    #[test]
    fn failed_registry_persistence_does_not_publish_or_cache_an_import() {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelStore::new(directory.path().to_path_buf()).unwrap();
        let model = imported_manifest();
        let target = store.path_for(&model);
        fs::write(&target, vec![5_u8; model.download_bytes as usize]).unwrap();
        let receipt = store.receipt_path(&model);
        write_receipt(&receipt, &model.sha256).unwrap();
        let fingerprint = file_fingerprint(&target).unwrap();
        fs::create_dir(directory.path().join(IMPORTED_MODELS_FILE)).unwrap();

        let error = store
            .finish_imported_model(
                &model,
                fingerprint,
                true,
                ImportArtifacts {
                    receipt: &receipt,
                    target: &target,
                    wrote_receipt: true,
                    installed_fresh_copy: true,
                },
            )
            .unwrap_err();

        assert!(error.contains("could not save imported model metadata"));
        assert!(store.resolve(&model.id).is_none());
        assert!(!store.verified.lock().unwrap().contains_key(&model.id));
        assert!(!target.exists());
        assert!(!receipt.exists());
    }

    #[test]
    fn failed_metadata_refresh_preserves_an_existing_installed_model() {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelStore::new(directory.path().to_path_buf()).unwrap();
        let existing = imported_manifest();
        let mut refreshed = existing.as_ref().clone();
        refreshed.quantization = WhisperQuantization::Q8_0;
        refreshed.display_name = imported_display_name(
            &WhisperModelInspection {
                family: refreshed.family,
                languages: refreshed.languages,
                quantization: refreshed.quantization.clone(),
            },
            &refreshed.sha256,
        );
        let refreshed = Arc::new(refreshed);
        store
            .imported
            .lock()
            .unwrap()
            .insert(existing.id.clone(), Arc::clone(&existing));

        let target = store.path_for(&existing);
        let target_bytes = vec![7_u8; existing.download_bytes as usize];
        fs::write(&target, &target_bytes).unwrap();
        let receipt = store.receipt_path(&existing);
        write_receipt(&receipt, &existing.sha256).unwrap();
        let receipt_bytes = fs::read(&receipt).unwrap();
        let fingerprint = file_fingerprint(&target).unwrap();
        store
            .verified
            .lock()
            .unwrap()
            .insert(existing.id.clone(), fingerprint.clone());
        fs::create_dir(directory.path().join(IMPORTED_MODELS_FILE)).unwrap();

        let error = store
            .finish_imported_model(
                &refreshed,
                fingerprint.clone(),
                true,
                ImportArtifacts {
                    receipt: &receipt,
                    target: &target,
                    wrote_receipt: false,
                    installed_fresh_copy: false,
                },
            )
            .unwrap_err();

        assert!(error.contains("could not save imported model metadata"));
        assert_eq!(
            store.resolve(&existing.id).as_deref(),
            Some(existing.as_ref())
        );
        assert_eq!(fs::read(&target).unwrap(), target_bytes);
        assert_eq!(fs::read(&receipt).unwrap(), receipt_bytes);
        assert_eq!(
            store.verified.lock().unwrap().get(&existing.id),
            Some(&fingerprint)
        );
    }

    #[test]
    fn incomplete_import_cleanup_is_reported_with_the_primary_error() {
        let directory = tempfile::tempdir().unwrap();
        let receipt = directory.path().join("receipt-directory");
        let target = directory.path().join("target-directory");
        fs::create_dir(&receipt).unwrap();
        fs::create_dir(&target).unwrap();

        let error = ImportArtifacts {
            receipt: &receipt,
            target: &target,
            wrote_receipt: true,
            installed_fresh_copy: true,
        }
        .rollback_after("registry persistence failed".into());

        assert!(error.contains("registry persistence failed"));
        assert!(error.contains("could not fully clean up"));
        assert!(error.contains("receipt-directory"));
        assert!(error.contains("target-directory"));
        assert!(receipt.is_dir());
        assert!(target.is_dir());
    }

    #[test]
    fn imported_registry_is_the_final_fallible_commit_step() {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelStore::new(directory.path().to_path_buf()).unwrap();
        let model = imported_manifest();
        let target = store.path_for(&model);
        fs::write(&target, vec![3_u8; model.download_bytes as usize]).unwrap();
        write_receipt(&store.receipt_path(&model), &model.sha256).unwrap();
        let fingerprint = file_fingerprint(&target).unwrap();

        store
            .commit_imported_model(&model, fingerprint.clone(), true)
            .unwrap();

        assert_eq!(store.resolve(&model.id).as_deref(), Some(model.as_ref()));
        assert_eq!(
            store.verified.lock().unwrap().get(&model.id),
            Some(&fingerprint)
        );
        let restarted = ModelStore::new(directory.path().to_path_buf()).unwrap();
        assert_eq!(
            restarted.resolve(&model.id).as_deref(),
            Some(model.as_ref())
        );
        assert_eq!(
            restarted
                .catalog(&model.id)
                .into_iter()
                .find(|summary| summary.manifest.id == model.id)
                .unwrap()
                .state,
            ModelInstallationState::Installed
        );
    }

    #[test]
    fn imported_display_names_come_from_inspection_and_digest() {
        let inspection = WhisperModelInspection {
            family: crate::engines::WhisperModelFamily::Small,
            languages: ModelLanguageSet::Multilingual,
            quantization: WhisperQuantization::Q5_1,
        };
        assert_eq!(
            imported_display_name(
                &inspection,
                "a1b2c3d4eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
            ),
            "Imported Small Q5_1 · a1b2c3d4"
        );
    }

    #[test]
    fn malformed_import_metadata_does_not_disable_curated_models_or_get_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let registry = directory.path().join(IMPORTED_MODELS_FILE);
        fs::write(&registry, b"not json").unwrap();

        let store = ModelStore::new(directory.path().to_path_buf()).unwrap();
        assert_eq!(
            store.catalog("unknown").len(),
            curated_whisper_models().len()
        );
        assert!(store
            .import_from_path(Path::new("unused.bin"))
            .unwrap_err()
            .contains("metadata needs attention"));
        assert_eq!(fs::read(&registry).unwrap(), b"not json");
    }

    #[test]
    fn import_preflight_rejects_gguf_and_unsafe_ggml_dimensions() {
        let directory = tempfile::tempdir().unwrap();
        let gguf = directory.path().join("model.gguf");
        let mut gguf_bytes = vec![0_u8; 48];
        gguf_bytes[..4].copy_from_slice(b"GGUF");
        fs::write(&gguf, gguf_bytes).unwrap();
        assert!(validate_import_source(&gguf).unwrap_err().contains("GGUF"));

        let mut invalid = tempfile::tempfile().unwrap();
        let fields = [51_865_i32, 1_500, 384, 6, 7, 448, 384, 6, 4, 80, 1];
        invalid.write_all(b"lmgg").unwrap();
        for field in fields {
            invalid.write_all(&field.to_le_bytes()).unwrap();
        }
        assert!(validate_ggml_header(&mut invalid)
            .unwrap_err()
            .contains("unsupported Whisper dimensions"));
    }

    #[cfg(unix)]
    #[test]
    fn import_copy_does_not_follow_a_selected_symbolic_link() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("actual.bin");
        fs::write(&source, [0_u8; 48]).unwrap();
        let selected = directory.path().join("selected.bin");
        symlink(&source, &selected).unwrap();
        let mut output = tempfile::tempfile().unwrap();

        assert!(copy_import_source(&selected, &mut output)
            .unwrap_err()
            .contains("symbolic link"));
    }

    #[test]
    fn quarantined_models_are_visible_and_removed_with_the_model() {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelStore::new(directory.path().to_path_buf()).unwrap();
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
        let store = ModelStore::new(directory.path().to_path_buf()).unwrap();
        let model = &curated_whisper_models()[0];
        let registration = store.begin_download(&model.id).unwrap();

        assert!(store.cancel_download(&model.id).unwrap());
        assert!(registration.cancellation.load(Ordering::Relaxed));
        drop(registration);
        assert!(!store.cancel_download(&model.id).unwrap());
    }

    #[test]
    #[ignore = "requires SPICK_WHISPER_MODEL_PATH"]
    fn real_model_import_survives_restart_and_can_be_removed() {
        let source = PathBuf::from(std::env::var("SPICK_WHISPER_MODEL_PATH").unwrap());
        let directory = tempfile::tempdir().unwrap();
        let model_id = {
            let store = ModelStore::new(directory.path().to_path_buf()).unwrap();
            let imported = store.import_from_path(&source).unwrap();
            assert_eq!(imported.manifest.origin, WhisperModelOrigin::Imported);
            assert_eq!(imported.state, ModelInstallationState::Installed);
            assert!(store
                .verified_model_path(&imported.manifest.id)
                .unwrap()
                .is_file());
            let installed = store.path_for(&imported.manifest);
            fs::write(&installed, b"corrupt").unwrap();
            let repaired = store.import_from_path(&source).unwrap();
            assert_eq!(repaired.manifest.id, imported.manifest.id);
            assert!(store
                .verified_model_path(&repaired.manifest.id)
                .unwrap()
                .is_file());
            imported.manifest.id
        };

        let store = ModelStore::new(directory.path().to_path_buf()).unwrap();
        assert_eq!(store.resolve(&model_id).unwrap().id, model_id);
        assert!(store
            .catalog(&model_id)
            .iter()
            .any(|summary| summary.manifest.id == model_id && summary.active));
        store.remove(&model_id).unwrap();
        assert!(store.resolve(&model_id).is_none());
    }
}
