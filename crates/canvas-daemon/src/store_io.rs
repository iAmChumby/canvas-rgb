//! Persisting the store without blocking a runtime worker (#241).
//!
//! A handler used to take `app.store.write()`, change it, and call a `save_*` method while
//! still holding the guard. Those methods do synchronous file I/O (temp file, flush, `sync_all`,
//! rename). A slow or stalled disk therefore held a Tokio worker *and* the store lock: every
//! reader, including `GET /api/v1/settings`, waited for the write to finish while the port kept
//! listening. That is the "Daemon connecting" hang in #241.
//!
//! The helpers here split the old block in two. [`store_save`] and [`store_save_rolled`] change
//! memory under the store write lock, then drop the lock and write a snapshot on
//! `spawn_blocking`. [`store_publish`] does the reverse for routes that must never show unsaved
//! state: it changes a copy, writes the copy, and swaps it in only after the write succeeded.
//! [`store_edit`] covers the mutations that do not write at all.
//!
//! Three rules hold for every helper, and `docs/bug-classes.md` (`cause:blocking-io`) names them:
//!
//! 1. **Read-modify-write is atomic.** The per-App writer mutex is taken first and held to the
//!    commit, and the closure that decides what to change runs after it. A route finds its target
//!    (a light by id, a preset by name, the `expected` props) *inside* that closure, never before
//!    it, so two requests cannot both start from the same old value.
//! 2. **A save outlives its caller.** Once the mutation has run, the write, the commit and the
//!    rollback run in a spawned task that owns the writer guard. A client that drops mid-save
//!    (the editor aborts a fetch during exactly the stall this module is for) cannot leave memory
//!    changed and disk not, and a following save cannot start while that write is still on its
//!    blocking thread: no two writes to one target overlap.
//! 3. **A reader never waits for the disk.** The store lock is held only for the in-memory change
//!    and the commit. The price is a read window: after [`store_save`] or [`store_save_rolled`]
//!    changes memory and before the write lands, a `GET` can see state that a failed save then
//!    rolls back. Routes whose state acts on the outside world ([`store_publish`]: displays,
//!    peripheral access, light patches) take the other path, so unsaved state is never visible.

use std::sync::Arc;

use crate::api::ApiFailure;
use crate::state::App;
use crate::storage::Store;
use std::sync::atomic::{AtomicUsize, Ordering};

/// How many detached store saves have started. A diagnostic so a poll path can prove it never
/// asks the writer for work when there is nothing to write (#580 C6).
static SAVE_COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn reset_save_count() {
    SAVE_COUNT.store(0, Ordering::SeqCst);
}

pub fn save_count() -> usize {
    SAVE_COUNT.load(Ordering::SeqCst)
}

/// What a caller sees when the detached save task itself died (a panic, a cancelled runtime).
fn task_failed(e: &tokio::task::JoinError) -> String {
    format!("the store write did not finish: {e}")
}

/// Changes the store, then writes a snapshot off the runtime worker with the store lock free.
///
/// `mutate` runs under the writer mutex and the store write lock, so it may read the store and
/// decide what to change in one step. `save` runs on a blocking thread against a snapshot taken
/// just after `mutate`, and reports its own failure so each caller keeps its current error text
/// and its current memory (a route with no rollback keeps the change it made).
pub(crate) async fn store_save<T: Send + 'static>(
    app: &Arc<App>,
    mutate: impl FnOnce(&mut Store) -> Result<T, ApiFailure>,
    save: impl FnOnce(&mut Store) -> Result<(), ApiFailure> + Send + 'static,
) -> Result<T, ApiFailure> {
    SAVE_COUNT.fetch_add(1, Ordering::SeqCst);
    let writer = Arc::clone(&app.store_writer).lock_owned().await;
    let (value, mut snapshot) = {
        let mut store = app.store.write().await;
        let value = mutate(&mut store)?;
        (value, store.clone())
    };
    // No await between the mutation above and the spawn below: a caller cannot be cancelled
    // between "memory changed" and "the write is in a task that will finish the job".
    let app = Arc::clone(app);
    let task = tokio::spawn(async move {
        let _writer = writer;
        let written = tokio::task::spawn_blocking(move || {
            let result = save(&mut snapshot);
            (result, snapshot)
        })
        .await;
        match written {
            Ok((result, snapshot)) => {
                *app.store.write().await = snapshot;
                result.map(|()| value)
            }
            Err(e) => Err(ApiFailure::internal(task_failed(&e))),
        }
    });
    task.await.unwrap_or_else(|e| Err(ApiFailure::internal(task_failed(&e))))
}

/// Like [`store_save`], with a rollback for a failed write.
///
/// `mutate` returns the answer and a context (the old values it put aside). If the write fails,
/// `failed` puts the context back under the store write lock and builds the route's error. The
/// rollback runs in the detached task, so a caller that went away still gets it.
pub(crate) async fn store_save_rolled<T: Send + 'static, C: Send + 'static>(
    app: &Arc<App>,
    mutate: impl FnOnce(&mut Store) -> Result<(T, C), ApiFailure>,
    save: impl FnOnce(&mut Store) -> std::io::Result<()> + Send + 'static,
    failed: impl FnOnce(C, &mut Store, std::io::Error, &T) -> ApiFailure + Send + 'static,
) -> Result<T, ApiFailure> {
    let writer = Arc::clone(&app.store_writer).lock_owned().await;
    let (value, context, mut snapshot) = {
        let mut store = app.store.write().await;
        let (value, context) = mutate(&mut store)?;
        (value, context, store.clone())
    };
    let app = Arc::clone(app);
    let task = tokio::spawn(async move {
        let _writer = writer;
        let written = tokio::task::spawn_blocking(move || {
            let result = save(&mut snapshot);
            (result, snapshot)
        })
        .await;
        let error = match written {
            Ok((Ok(()), snapshot)) => {
                *app.store.write().await = snapshot;
                None
            }
            Ok((Err(e), _)) => Some(e),
            Err(e) => Some(std::io::Error::other(task_failed(&e))),
        };
        if let Some(error) = error {
            let mut store = app.store.write().await;
            return Err(failed(context, &mut store, error, &value));
        }
        Ok(value)
    });
    task.await.unwrap_or_else(|e| Err(ApiFailure::internal(task_failed(&e))))
}

/// Changes a copy of the store, writes the copy, and publishes it only after the write succeeded.
///
/// `edit` runs on a clone, under the writer mutex, and reads whatever it needs from that clone.
/// `save` runs on a blocking thread against the changed clone and the answer `edit` gave. When
/// `save` fails the live store was never touched, so there is nothing to roll back and no reader
/// ever saw the change. Use it where memory ahead of the disk is wrong: state the daemon acts on
/// (peripheral access, display configs, a light's arrangement).
pub(crate) async fn store_publish<T: Send + 'static>(
    app: &Arc<App>,
    edit: impl FnOnce(&mut Store) -> Result<T, ApiFailure>,
    save: impl FnOnce(&Store, &T) -> Result<(), ApiFailure> + Send + 'static,
) -> Result<T, ApiFailure> {
    let writer = Arc::clone(&app.store_writer).lock_owned().await;
    let mut next = app.store.read().await.clone();
    let value = edit(&mut next)?;
    let app = Arc::clone(app);
    let task = tokio::spawn(async move {
        let _writer = writer;
        let written = tokio::task::spawn_blocking(move || {
            let result = save(&next, &value);
            (result, next, value)
        })
        .await;
        match written {
            Ok((Ok(()), next, value)) => {
                *app.store.write().await = next;
                Ok(value)
            }
            Ok((Err(e), _, _)) => Err(e),
            Err(e) => Err(ApiFailure::internal(task_failed(&e))),
        }
    });
    task.await.unwrap_or_else(|e| Err(ApiFailure::internal(task_failed(&e))))
}

/// A store mutation that does no file I/O, serialised with the save helpers.
///
/// It takes the writer mutex first, the same order as the saves, so a change made here cannot be
/// overwritten when a save that snapshotted an older store commits.
pub(crate) async fn store_edit<T>(app: &Arc<App>, edit: impl FnOnce(&mut Store) -> T) -> T {
    let _writer = app.store_writer.lock().await;
    edit(&mut *app.store.write().await)
}
