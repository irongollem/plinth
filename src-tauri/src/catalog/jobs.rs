//! One registry and one policy for the catalog's background work.
//!
//! Every job that touches the catalog claims a permit here before it
//! starts, and the permit's lifetime is the job's: dropping it — on
//! success, on failure, on cancellation, or on a panic unwinding out of
//! the worker thread — releases the claim and wakes whatever was waiting
//! for it.

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Scan,
    Reclassify,
    Duplicate,
    Geometry,
    Pack,
    Unpack,
    Extract,
    BatchRender,
}

/// What a job does to the catalog — all the coordinator needs in order to
/// keep two of them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    /// Rewrites catalog rows. SQLite admits one writer at a time, and two
    /// of these also disagree about what the rows should say: a scan's
    /// replace_catalog rewrites the very rows a dedupe is hashing into.
    WritesCatalog,
    /// Moves or deletes the bytes another job is reading, so it excludes
    /// even the jobs that never write a row.
    MovesBytes,
    /// Reads catalog rows and writes only scratch files of its own.
    ReadsFiles,
}

impl JobKind {
    fn access(self) -> Access {
        match self {
            JobKind::Scan
            | JobKind::Reclassify
            | JobKind::Duplicate
            | JobKind::Geometry
            | JobKind::BatchRender => Access::WritesCatalog,
            JobKind::Pack | JobKind::Unpack => Access::MovesBytes,
            JobKind::Extract => Access::ReadsFiles,
        }
    }

    /// Names the holder in "X is running" refusals.
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Scan => "A catalog scan",
            JobKind::Reclassify => "A designer reclassification",
            JobKind::Duplicate => "A duplicate scan",
            JobKind::Geometry => "A geometry scan",
            JobKind::Pack => "A pack job",
            JobKind::Unpack => "An unpack job",
            JobKind::Extract => "An extraction",
            JobKind::BatchRender => "A batch render",
        }
    }

    /// Completes "… when it finishes" for the job that was turned away.
    fn retry_hint(self) -> &'static str {
        match self {
            JobKind::Scan => "rescan",
            JobKind::Reclassify => "reclassify designers",
            JobKind::Duplicate => "scan for duplicates",
            JobKind::Geometry => "mine geometry",
            JobKind::Pack => "pack",
            JobKind::Unpack => "unpack",
            JobKind::Extract => "open packed files",
            JobKind::BatchRender => "render previews",
        }
    }

    /// Job ids carry their kind as a prefix. The frontend cancels by id
    /// and the ids reach it through events, so these are wire format.
    fn prefix(self) -> &'static str {
        match self {
            JobKind::Scan => "scan:",
            JobKind::Reclassify => "reclassify:",
            JobKind::Duplicate => "dup:",
            JobKind::Geometry => "geom:",
            // one prefix for both directions: they are the same exclusion
            // to everything else, and the frontend cancels them alike
            JobKind::Pack | JobKind::Unpack => "pack:",
            JobKind::Extract => "extract:",
            JobKind::BatchRender => "batch-render:",
        }
    }
}

/// Whether two jobs may run at the same time. Read-only catalog queries
/// never come through here — WAL lets them run alongside anything.
fn conflicts(holder: Access, want: Access) -> bool {
    match (holder, want) {
        (Access::MovesBytes, _) | (_, Access::MovesBytes) => true,
        (Access::WritesCatalog, Access::WritesCatalog) => true,
        (Access::ReadsFiles, _) | (_, Access::ReadsFiles) => false,
    }
}

struct Active {
    kind: JobKind,
    cancel: Arc<AtomicBool>,
}

static ACTIVE: Lazy<Mutex<HashMap<String, Active>>> = Lazy::new(|| Mutex::new(HashMap::new()));
/// Woken every time a permit is dropped, so queued work can re-try its
/// claim instead of polling.
static RELEASED: Lazy<Notify> = Lazy::new(Notify::new);

/// A job's claim on the catalog. Holding one is what makes the job
/// active; dropping it is what ends it, however the job ended.
#[derive(Debug)]
pub struct JobPermit {
    id: String,
    cancel: Arc<AtomicBool>,
}

impl JobPermit {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }
}

impl Drop for JobPermit {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE.lock() {
            active.remove(&self.id);
        }
        RELEASED.notify_waiters();
    }
}

fn busy_error(holder: JobKind, want: JobKind) -> AppError {
    AppError::InvalidInput(if holder == want {
        format!("{} is already running", holder.label())
    } else {
        format!(
            "{} is running — {} when it finishes",
            holder.label(),
            want.retry_hint()
        )
    })
}

/// Claim the catalog for `kind`, or report who holds it. The conflict
/// check and the registration happen under one lock: two jobs starting at
/// the same moment cannot both find the catalog free.
pub fn claim(kind: JobKind) -> Result<JobPermit, AppError> {
    let mut active = ACTIVE
        .lock()
        .map_err(|e| AppError::ConfigError(format!("Job registry unavailable: {}", e)))?;
    if let Some(holder) = active
        .values()
        .find(|job| conflicts(job.kind.access(), kind.access()))
    {
        return Err(busy_error(holder.kind, kind));
    }
    let id = format!("{}{}", kind.prefix(), Uuid::new_v4());
    let cancel = Arc::new(AtomicBool::new(false));
    active.insert(
        id.clone(),
        Active {
            kind,
            cancel: Arc::clone(&cancel),
        },
    );
    Ok(JobPermit { id, cancel })
}

/// Wait for the catalog rather than refusing it — for follow-up work the
/// user didn't ask for directly and won't think to retry. A several-hour
/// dedupe must not be lost to a routine refresh after an import.
pub async fn claim_when_free(kind: JobKind) -> Result<JobPermit, AppError> {
    loop {
        // Arm the wakeup BEFORE re-checking, or a permit dropped between
        // the check and the await would leave this waiting for a release
        // that has already happened.
        let released = RELEASED.notified();
        tokio::pin!(released);
        released.as_mut().enable();
        match claim(kind) {
            Ok(permit) => return Ok(permit),
            Err(AppError::InvalidInput(_)) => released.await,
            Err(other) => return Err(other),
        }
    }
}

/// Ask a running job to stop. The job owns when it notices.
pub fn cancel(job_id: &str) -> bool {
    match ACTIVE.lock() {
        Ok(active) => match active.get(job_id) {
            Some(job) => {
                job.cancel.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        },
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry is process-wide, so tests that claim must not run
    /// concurrently with each other.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn exclusive_guard() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn a_second_catalog_writer_is_refused_by_name() {
        let _serial = exclusive_guard();
        let scan = claim(JobKind::Scan).unwrap();

        let err = claim(JobKind::Duplicate).unwrap_err();
        assert!(err
            .to_string()
            .ends_with("A catalog scan is running — scan for duplicates when it finishes"));
        // …and the reverse pairing, plus scan against itself
        assert!(claim(JobKind::Geometry).is_err());
        assert!(claim(JobKind::Scan).is_err());

        drop(scan);
        assert!(claim(JobKind::Duplicate).is_ok());
    }

    #[test]
    fn reading_files_overlaps_a_writer_but_not_a_mover() {
        let _serial = exclusive_guard();
        let dedupe = claim(JobKind::Duplicate).unwrap();
        // opening a packed model only reads rows: it need not wait hours
        let extract = claim(JobKind::Extract).unwrap();
        drop(extract);
        drop(dedupe);

        let pack = claim(JobKind::Pack).unwrap();
        // …but packing deletes the very bytes an extraction is reading
        assert!(claim(JobKind::Extract).is_err());
        drop(pack);
    }

    #[test]
    fn a_panicking_job_still_releases_the_catalog() {
        let _serial = exclusive_guard();
        let panicked = std::thread::spawn(|| {
            let _permit = claim(JobKind::Scan).unwrap();
            panic!("job exploded");
        })
        .join();
        assert!(panicked.is_err());

        // the permit unwound with the thread, so the catalog is free
        assert!(claim(JobKind::Scan).is_ok());
    }

    // holding the serial guard across the awaits is the point: this test
    // owns the process-wide registry for its duration
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn queued_work_waits_for_the_holder_instead_of_failing() {
        let _serial = exclusive_guard();
        let dedupe = claim(JobKind::Duplicate).unwrap();
        let queued = tokio::spawn(claim_when_free(JobKind::Scan));

        // still held: the queued scan has not started
        tokio::task::yield_now().await;
        assert!(!queued.is_finished());

        drop(dedupe);
        let scan = queued.await.unwrap().unwrap();
        assert!(scan.id().starts_with("scan:"));
    }

    #[test]
    fn cancelling_sets_the_flag_the_job_polls() {
        let _serial = exclusive_guard();
        let permit = claim(JobKind::Geometry).unwrap();
        let flag = permit.cancel_flag();
        assert!(!flag.load(Ordering::SeqCst));
        assert!(cancel(permit.id()));
        assert!(flag.load(Ordering::SeqCst));

        let id = permit.id().to_string();
        drop(permit);
        // a finished job's id no longer cancels anything
        assert!(!cancel(&id));
    }
}
