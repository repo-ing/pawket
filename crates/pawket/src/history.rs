//! A small, local record of what the engine has saved for the browser.
//!
//! Kept as one JSON file next to the transfer folder. It stores titles and
//! source links only; the files themselves live wherever the browser put them.

use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{fs, sync::Mutex};

pub const MAX_ENTRIES: usize = 50;
const FILE_NAME: &str = "history.json";
const MAX_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub title: String,
    pub source_url: String,
    pub filename: String,
    pub bytes: u64,
    /// Seconds since the Unix epoch.
    pub saved_at: u64,
    #[serde(default)]
    pub provider: String,
}

impl HistoryEntry {
    pub fn now(title: &str, source_url: &str, filename: &str, bytes: u64, provider: &str) -> Self {
        let saved_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        Self {
            title: title.to_owned(),
            source_url: source_url.to_owned(),
            filename: filename.to_owned(),
            bytes,
            saved_at,
            provider: provider.to_owned(),
        }
    }
}

/// Newest-first list of saved files, persisted after every change.
pub struct History {
    path: PathBuf,
    entries: Mutex<Vec<HistoryEntry>>,
}

impl History {
    /// Load the history kept in `directory`. A missing, oversized or
    /// unreadable file starts an empty history rather than failing startup.
    pub async fn load(directory: &Path) -> Self {
        let path = directory.join(FILE_NAME);
        let entries = read_entries(&path).await.unwrap_or_default();
        Self {
            path,
            entries: Mutex::new(entries),
        }
    }

    pub async fn list(&self) -> Vec<HistoryEntry> {
        self.entries.lock().await.clone()
    }

    /// Add `entry` at the top, keep the newest `MAX_ENTRIES`, and persist.
    pub async fn record(&self, entry: HistoryEntry) -> std::io::Result<()> {
        let mut entries = self.entries.lock().await;
        let updated = with_entry(&entries, entry);
        write_entries(&self.path, &updated).await?;
        *entries = updated;
        Ok(())
    }

    pub async fn clear(&self) -> std::io::Result<()> {
        let mut entries = self.entries.lock().await;
        write_entries(&self.path, &[]).await?;
        *entries = Vec::new();
        Ok(())
    }
}

fn with_entry(existing: &[HistoryEntry], entry: HistoryEntry) -> Vec<HistoryEntry> {
    std::iter::once(entry)
        .chain(existing.iter().cloned())
        .take(MAX_ENTRIES)
        .collect()
}

async fn read_entries(path: &Path) -> Option<Vec<HistoryEntry>> {
    let size = fs::metadata(path).await.ok()?.len();
    if size > MAX_FILE_BYTES {
        return None;
    }
    let data = fs::read(path).await.ok()?;
    let entries: Vec<HistoryEntry> = serde_json::from_slice(&data).ok()?;
    Some(entries.into_iter().take(MAX_ENTRIES).collect())
}

/// Write to a sibling temporary file and rename, so a crash mid-write
/// never leaves a half-written history.
async fn write_entries(path: &Path, entries: &[HistoryEntry]) -> std::io::Result<()> {
    let data = serde_json::to_vec_pretty(entries).map_err(std::io::Error::other)?;
    let temporary = path.with_extension("json.part");
    fs::write(&temporary, data).await?;
    fs::rename(&temporary, path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: u64) -> HistoryEntry {
        HistoryEntry {
            title: format!("Clip {n}"),
            source_url: format!("https://example.org/{n}"),
            filename: format!("clip-{n}.mp4"),
            bytes: n,
            saved_at: n,
            provider: "direct_https".into(),
        }
    }

    #[tokio::test]
    async fn records_newest_first_and_survives_a_restart() {
        let directory = tempfile::tempdir().unwrap();
        let history = History::load(directory.path()).await;
        assert!(history.list().await.is_empty());
        history.record(entry(1)).await.unwrap();
        history.record(entry(2)).await.unwrap();

        let reloaded = History::load(directory.path()).await;
        assert_eq!(reloaded.list().await, vec![entry(2), entry(1)]);
    }

    #[tokio::test]
    async fn keeps_only_the_newest_entries() {
        let directory = tempfile::tempdir().unwrap();
        let history = History::load(directory.path()).await;
        for n in 0..(MAX_ENTRIES as u64 + 5) {
            history.record(entry(n)).await.unwrap();
        }
        let entries = history.list().await;
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries[0], entry(MAX_ENTRIES as u64 + 4));
    }

    #[tokio::test]
    async fn clear_empties_memory_and_disk() {
        let directory = tempfile::tempdir().unwrap();
        let history = History::load(directory.path()).await;
        history.record(entry(1)).await.unwrap();
        history.clear().await.unwrap();
        assert!(history.list().await.is_empty());
        assert!(History::load(directory.path())
            .await
            .list()
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn a_corrupt_file_starts_empty_instead_of_failing() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join(FILE_NAME), b"{not json").unwrap();
        let history = History::load(directory.path()).await;
        assert!(history.list().await.is_empty());
        history.record(entry(7)).await.unwrap();
        assert_eq!(history.list().await, vec![entry(7)]);
    }
}
