//! Background download jobs. A job keeps running in the engine after the
//! page or popup that started it closes, and reports its progress when asked.

use pawket_core::Progress;
use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const MAX_JOBS: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    /// Waiting for one of the engine's download slots.
    Queued,
    Running,
    Done,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct JobError {
    pub code: String,
    pub message: String,
}

struct Job {
    id: String,
    title: String,
    source_url: String,
    label: Option<String>,
    client: String,
    status: JobStatus,
    progress: Arc<Progress>,
    filename: Option<String>,
    file_url: Option<String>,
    error: Option<JobError>,
    delivered: bool,
    finished: Option<Instant>,
}

/// What clients see for one job.
#[derive(Clone, Debug, Serialize)]
pub struct JobView {
    pub id: String,
    pub title: String,
    pub source_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Which surface started the job ("web", "extension", …). A surface
    /// only shows and collects its own jobs.
    pub client: String,
    pub status: JobStatus,
    pub bytes_done: u64,
    /// `null` when the source did not state a size.
    pub bytes_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// Ticketed link to the prepared file, once the job is done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
    /// True once a browser has taken the finished file.
    pub delivered: bool,
}

impl Job {
    fn view(&self) -> JobView {
        JobView {
            id: self.id.clone(),
            title: self.title.clone(),
            source_url: self.source_url.clone(),
            label: self.label.clone(),
            client: self.client.clone(),
            status: self.status,
            bytes_done: self.progress.written(),
            bytes_total: self.progress.total(),
            filename: self.filename.clone(),
            url: self.file_url.clone(),
            error: self.error.clone(),
            delivered: self.delivered,
        }
    }

    fn is_active(&self) -> bool {
        matches!(self.status, JobStatus::Queued | JobStatus::Running)
    }
}

/// Newest-first list of jobs. Finished jobs expire with their prepared file.
pub struct JobStore {
    jobs: Mutex<Vec<Job>>,
    finished_ttl: Duration,
}

impl JobStore {
    pub fn new(finished_ttl: Duration) -> Self {
        Self {
            jobs: Mutex::new(Vec::new()),
            finished_ttl,
        }
    }

    /// Queue a job and return the counters its download should update.
    pub fn add(
        &self,
        id: &str,
        title: &str,
        source_url: &str,
        label: Option<&str>,
        client: &str,
    ) -> Arc<Progress> {
        let progress = Arc::new(Progress::default());
        let job = Job {
            id: id.to_owned(),
            title: title.to_owned(),
            source_url: source_url.to_owned(),
            label: label.map(str::to_owned),
            client: client.to_owned(),
            status: JobStatus::Queued,
            progress: progress.clone(),
            filename: None,
            file_url: None,
            error: None,
            delivered: false,
            finished: None,
        };
        let mut jobs = self.lock();
        jobs.insert(0, job);
        prune(&mut jobs, self.finished_ttl);
        progress
    }

    pub fn start(&self, id: &str) {
        self.update(id, |job| job.status = JobStatus::Running);
    }

    pub fn finish(&self, id: &str, filename: &str, file_url: &str) {
        self.update(id, |job| {
            job.status = JobStatus::Done;
            job.filename = Some(filename.to_owned());
            job.file_url = Some(file_url.to_owned());
            job.finished = Some(Instant::now());
        });
    }

    pub fn fail(&self, id: &str, code: &str, message: &str) {
        self.update(id, |job| {
            job.status = JobStatus::Failed;
            job.error = Some(JobError {
                code: code.to_owned(),
                message: message.to_owned(),
            });
            job.finished = Some(Instant::now());
        });
    }

    /// Claim a finished file for one browser. `Some(true)` for the first
    /// caller, `Some(false)` if it was already taken, `None` if unknown.
    /// Claiming first keeps two open windows from both downloading it.
    pub fn claim_delivery(&self, id: &str) -> Option<bool> {
        let mut jobs = self.lock();
        let job = jobs.iter_mut().find(|job| job.id == id)?;
        let is_first = !job.delivered;
        job.delivered = true;
        Some(is_first)
    }

    /// Forget finished jobs. Jobs still downloading are kept.
    pub fn clear_finished(&self) {
        self.lock().retain(Job::is_active);
    }

    pub fn list(&self) -> Vec<JobView> {
        let mut jobs = self.lock();
        prune(&mut jobs, self.finished_ttl);
        jobs.iter().map(Job::view).collect()
    }

    fn update(&self, id: &str, change: impl FnOnce(&mut Job)) -> bool {
        let mut jobs = self.lock();
        match jobs.iter_mut().find(|job| job.id == id) {
            Some(job) => {
                change(job);
                true
            }
            None => false,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Job>> {
        // A panic while holding this lock cannot leave the list half-updated
        // in a harmful way, so keep serving the data.
        self.jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Drop expired finished jobs, then the oldest finished ones beyond the cap.
/// Active jobs are never dropped.
fn prune(jobs: &mut Vec<Job>, finished_ttl: Duration) {
    jobs.retain(|job| {
        job.finished
            .is_none_or(|finished| finished.elapsed() < finished_ttl)
    });
    while jobs.len() > MAX_JOBS {
        let Some(oldest_finished) = jobs.iter().rposition(|job| !job.is_active()) else {
            break;
        };
        jobs.remove(oldest_finished);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(60);

    #[test]
    fn a_job_moves_from_queued_to_done_and_reports_progress() {
        let store = JobStore::new(TTL);
        let progress = store.add("a", "Clip", "https://example.org/a", Some("720p"), "web");
        let queued = &store.list()[0];
        assert_eq!(queued.status, JobStatus::Queued);
        assert_eq!((queued.bytes_done, queued.bytes_total), (0, None));
        assert_eq!(queued.label.as_deref(), Some("720p"), "web");

        store.start("a");
        assert_eq!(store.list()[0].status, JobStatus::Running);
        assert!(Arc::ptr_eq(&progress, &store.lock()[0].progress));

        store.finish("a", "clip.mp4", "/v1/files/a?ticket=t");
        let done = &store.list()[0];
        assert_eq!(done.status, JobStatus::Done);
        assert_eq!(done.filename.as_deref(), Some("clip.mp4"));
        assert_eq!(done.url.as_deref(), Some("/v1/files/a?ticket=t"));
        assert!(!done.delivered);
        assert_eq!(store.claim_delivery("a"), Some(true));
        assert!(store.list()[0].delivered);
        assert_eq!(store.claim_delivery("a"), Some(false));
        assert_eq!(store.claim_delivery("missing"), None);
        assert_eq!(store.list()[0].client, "web");
    }

    #[test]
    fn failures_carry_a_code_and_message() {
        let store = JobStore::new(TTL);
        store.add("a", "Clip", "https://example.org/a", None, "web");
        store.fail("a", "too_large", "The file is too large.");
        let failed = &store.list()[0];
        assert_eq!(failed.status, JobStatus::Failed);
        assert_eq!(failed.error.as_ref().unwrap().code, "too_large");
    }

    #[test]
    fn newest_first_and_only_finished_jobs_are_dropped() {
        let store = JobStore::new(TTL);
        store.add("running", "Keep", "https://example.org/k", None, "web");
        store.start("running");
        for n in 0..MAX_JOBS + 5 {
            let id = format!("done-{n}");
            store.add(&id, "Old", "https://example.org/o", None, "web");
            store.finish(&id, "o.mp4", "/v1/files/o?ticket=t");
        }
        let jobs = store.list();
        assert_eq!(jobs.len(), MAX_JOBS);
        assert_eq!(jobs[0].id, format!("done-{}", MAX_JOBS + 4));
        assert!(jobs.iter().any(|job| job.id == "running"));
    }

    #[test]
    fn finished_jobs_expire_and_can_be_cleared() {
        let store = JobStore::new(Duration::ZERO);
        store.add("a", "Clip", "https://example.org/a", None, "web");
        store.finish("a", "a.mp4", "/v1/files/a?ticket=t");
        assert!(store.list().is_empty());

        let store = JobStore::new(TTL);
        store.add("active", "Clip", "https://example.org/a", None, "web");
        store.add("done", "Clip", "https://example.org/a", None, "web");
        store.finish("done", "a.mp4", "/v1/files/a?ticket=t");
        store.clear_finished();
        let jobs = store.list();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "active");
    }
}
