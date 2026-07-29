//! Bounded parallel execution with input-order result collection.

use anyhow::{Context, Result};
use rayon::prelude::*;

pub fn map_ordered<T, R, F>(items: &[T], jobs: usize, operation: F) -> Result<Vec<R>>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync + Send,
{
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .context("failed to create worker pool")?;
    Ok(pool.install(|| items.par_iter().map(operation).collect()))
}
