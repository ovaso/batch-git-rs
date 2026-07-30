//! Bounded parallel execution with input-order result collection.

use anyhow::{Context, Result};
use rayon::prelude::*;

/// 使用指定数量的线程并行映射，同时保持结果与输入顺序一致。
pub fn map_ordered<T, R, F>(items: &[T], jobs: usize, operation: F) -> Result<Vec<R>>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync + Send,
{
    // 每次建立独立线程池，确保全局 Rayon 配置不会改变 `--jobs` 的语义。
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .context("failed to create worker pool")?;
    // IndexedParallelIterator 的 collect 会恢复输入顺序，使批量输出保持稳定。
    Ok(pool.install(|| items.par_iter().map(operation).collect()))
}
