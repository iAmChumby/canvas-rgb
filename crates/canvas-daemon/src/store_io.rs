//! Persisting the store without blocking a runtime worker (#241).
//!
//! A handler used to take `app.store.write()`, change it, and call a `save_*` method while
//! still holding the guard. Those methods do synchronous file I/O (temp file, flush, `sync_all`,
//! rename). A slow or stalled disk therefore held a Tokio worker *and* the store lock: every
//! reader, including `GET /api/v1/settings`, waited for the write to finish while the port kept
//! listening. That is the "Daemon connecting" hang in #241.
//!
//! The two helpers here split the old block in two. [`store_save`] (and [`store_save_rolled`],
//! for routes that put memory back on a failed save) change memory under the store write lock,
//! then drop the lock and write a snapshot on `spawn_blocking`. The mutation is still synchronous,
//! so a caller that is cancelled after it sees its own change in memory; only the disk write is
//! detached. [`store_edit`] covers the few mutations that do not write at all.
//!
//! A single per-App writer mutex is held from the mutation to the commit, so writes land one at
//! a time in the order callers asked for, and a store reader is never made to wait for the disk.

use std::sync::Arc;

use crate::api::ApiFailure;
use crate::state::App;
use crate::storage::Store;

/// Changes the store, then writes a snapshot off the runtime worker with the store lock free.
///
/// `mutate` runs under the store write lock. `save` runs on a blocking thread against a snapshot
/// taken just after `mutate`, and reports its own failure so each caller keeps its current error
/// text and its current memory (a route with no rollback keeps the change it made).
pub(crate) async fn store_save<T>(
    app: &Arc<App>,
    mutate: impl FnOnce(&mut Store) -> Result<T, ApiFailure>,
    save: impl FnOnce(&mut Store) -> Result<(), ApiFailure> + Send + 'static,
) -> Result<T, ApiFailure> {
    let _writer = app.store_writer.lock().await;
    let (value, mut snapshot) = {
        let mut store = app.store.write().await;
        let value = mutate(&mut store)?;
        (value, store.clone())
    };
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
        Err(e) => Err(ApiFailure::internal(format!("the store write did not finish: {e}"))),
    }
}

/// Like [`store_save`], with a rollback for a failed write.
///
/// `mutate` returns the answer and a context (the old values it put aside). If the write fails,
/// `failed` puts the context back under the store write lock and builds the route's error.
pub(crate) async fn store_save_rolled<T, C>(
    app: &Arc<App>,
    mutate: impl FnOnce(&mut Store) -> Result<(T, C), ApiFailure>,
    save: impl FnOnce(&mut Store) -> std::io::Result<()> + Send + 'static,
    failed: impl FnOnce(C, &mut Store, std::io::Error, &T) -> ApiFailure,
) -> Result<T, ApiFailure> {
    let _writer = app.store_writer.lock().await;
    let (value, context, mut snapshot) = {
        let mut store = app.store.write().await;
        let (value, context) = mutate(&mut store)?;
        (value, context, store.clone())
    };
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
        Err(e) => Some(std::io::Error::other(format!("the store write did not finish: {e}"))),
    };
    if let Some(error) = error {
        let mut store = app.store.write().await;
        return Err(failed(context, &mut store, error, &value));
    }
    Ok(value)
}

/// A store mutation that does no file I/O, serialised with the two save helpers.
///
/// It takes the writer mutex first, the same order as the saves, so a change made here cannot be
/// overwritten when a save that snapshotted an older store commits.
pub(crate) async fn store_edit<T>(app: &Arc<App>, edit: impl FnOnce(&mut Store) -> T) -> T {
    let _writer = app.store_writer.lock().await;
    edit(&mut *app.store.write().await)
}
