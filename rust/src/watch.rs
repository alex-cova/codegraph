use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::config;
use crate::directory;
use crate::extraction;

#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub files_changed: usize,
    pub duration_ms: u128,
}

pub struct WatcherHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WatcherHandle {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    pub fn is_active(&self) -> bool {
        !self.stop.load(Ordering::SeqCst)
    }
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn watch_disabled_reason(project_root: &Path) -> Option<String> {
    if std::env::var("CODEGRAPH_NO_WATCH").ok().as_deref() == Some("1") {
        return Some("CODEGRAPH_NO_WATCH=1 is set".to_string());
    }
    if std::env::var("CODEGRAPH_FORCE_WATCH").ok().as_deref() == Some("1") {
        return None;
    }
    if is_wsl() && is_windows_drive_mount(project_root) {
        return Some(
            "project is on a WSL2 /mnt/ drive, where recursive watch is too slow to be reliable"
                .to_string(),
        );
    }
    None
}

pub fn start_watcher<F, E>(
    project_root: PathBuf,
    debounce_ms: u64,
    on_sync_complete: F,
    on_sync_error: E,
) -> Result<WatcherHandle>
where
    F: Fn(WatchEvent) + Send + Sync + 'static,
    E: Fn(anyhow::Error) + Send + Sync + 'static,
{
    if let Some(reason) = watch_disabled_reason(&project_root) {
        anyhow::bail!(reason);
    }
    if !directory::is_initialized(&project_root) {
        anyhow::bail!("CodeGraph is not initialized in {}", project_root.display());
    }

    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);
    let on_sync_complete = Arc::new(on_sync_complete);
    let on_sync_error = Arc::new(on_sync_error);
    let thread_root = project_root.clone();

    let thread = thread::Builder::new()
        .name("codegraph-watch".to_string())
        .spawn(move || {
            let mut snapshot = match load_snapshot(&thread_root) {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    on_sync_error(err);
                    return;
                }
            };

            let poll_interval = Duration::from_millis(debounce_ms.max(500));
            while !stop_clone.load(Ordering::SeqCst) {
                thread::sleep(poll_interval);
                if stop_clone.load(Ordering::SeqCst) {
                    break;
                }

                let latest = match load_snapshot(&thread_root) {
                    Ok(snapshot) => snapshot,
                    Err(err) => {
                        on_sync_error(err);
                        continue;
                    }
                };

                let changed_count = diff_count(&snapshot, &latest);
                if changed_count == 0 {
                    snapshot = latest;
                    continue;
                }

                let started = std::time::Instant::now();
                let sync_result = (|| -> Result<usize> {
                    let config_value = config::load_config(&thread_root)?;
                    let summary = extraction::sync_project(&thread_root, &config_value)?;
                    Ok(summary.files_reindexed)
                })();

                match sync_result {
                    Ok(files_changed) => {
                        snapshot = match load_snapshot(&thread_root) {
                            Ok(snapshot) => snapshot,
                            Err(err) => {
                                on_sync_error(err);
                                latest
                            }
                        };
                        on_sync_complete(WatchEvent {
                            files_changed: files_changed.max(changed_count),
                            duration_ms: started.elapsed().as_millis(),
                        });
                    }
                    Err(err) => on_sync_error(err),
                }
            }
        })
        .context("failed to spawn watcher thread")?;

    Ok(WatcherHandle {
        stop,
        thread: Some(thread),
    })
}

fn load_snapshot(project_root: &Path) -> Result<BTreeMap<String, FileState>> {
    let config_value = config::load_config(project_root)?;
    let files = extraction::scan_directory(project_root, &config_value)?;
    let mut snapshot = BTreeMap::new();
    for path in files {
        let absolute = project_root.join(&path);
        let metadata = match std::fs::metadata(&absolute) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(system_time_ms)
            .unwrap_or_default();
        snapshot.insert(
            path,
            FileState {
                size: metadata.len(),
                modified_ms,
            },
        );
    }
    Ok(snapshot)
}

fn diff_count(
    previous: &BTreeMap<String, FileState>,
    current: &BTreeMap<String, FileState>,
) -> usize {
    let mut count = 0usize;
    for (path, state) in current {
        if previous.get(path) != Some(state) {
            count += 1;
        }
    }
    for path in previous.keys() {
        if !current.contains_key(path) {
            count += 1;
        }
    }
    count
}

fn system_time_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

fn is_windows_drive_mount(project_root: &Path) -> bool {
    let normalized = project_root.to_string_lossy().replace('\\', "/");
    let bytes = normalized.as_bytes();
    bytes.len() >= 6
        && &bytes[0..5].to_ascii_lowercase() == b"/mnt/"
        && bytes[5].is_ascii_alphabetic()
        && (bytes.len() == 6 || bytes[6] == b'/')
}

fn is_wsl() -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    if std::env::var_os("WSL_DISTRO_NAME").is_some() || std::env::var_os("WSL_INTEROP").is_some()
    {
        return true;
    }
    std::fs::read_to_string("/proc/version")
        .map(|version| {
            let lower = version.to_lowercase();
            lower.contains("microsoft") || lower.contains("wsl")
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileState {
    size: u64,
    modified_ms: u64,
}

