//! Stable, per-repository command results and rendering.

mod child_output;
mod jsonl;
mod machine;
mod result;
mod text;

pub(crate) use jsonl::JsonlProgress;
pub(crate) use result::RepositoryResult;
pub(crate) use text::{
    print_checkout_summary, print_operation_summary, print_push_summary, print_results,
    print_selected_results,
};
