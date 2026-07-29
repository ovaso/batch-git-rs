//! Built-in command orchestration.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, bail};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serde::Serialize;

use crate::cli::{
    CheckoutArgs, Cli, CloneArgs, Command, ExecArgs, FindArgs, ForgetArgs, InfoArgs, ListArgs,
    MergeArgs, PushArgs, RuntimeOptions, ScanArgs, SyncArgs,
};
use crate::color;
use crate::git::{
    self, BranchKind, CheckoutTarget, CloneOptions, GitOutput, RepositoryRuntimeState,
    UpstreamSummary,
};
use crate::model::{RepositoryRecord, WORKSPACE_FILE, Workspace, now, validate_directory};
use crate::parallel::map_ordered;
use crate::report::{
    RepositoryResult, print_checkout_summary, print_push_summary, print_results,
    print_selected_results,
};
use crate::settings;
use crate::table;
use crate::workspace::{self, WorkspaceLock};

pub fn dispatch(cli: Cli) -> Result<i32> {
    let jobs = settings::jobs(cli.jobs)?;
    match cli.command {
        Command::Clone(arguments) => clone_repository(arguments),
        Command::Scan(arguments) => scan(arguments, jobs),
        Command::Restore => restore(jobs, cli.verbose),
        Command::Fetch => fetch(jobs, cli.verbose),
        Command::Sync(arguments) => sync(arguments, jobs, cli.verbose),
        Command::Schedule(arguments) => crate::schedule::dispatch(arguments, jobs, cli.verbose),
        Command::Checkout(arguments) => checkout(arguments, jobs, cli.verbose),
        Command::Cd => checkout(checkout_alias_arguments(true, false), jobs, cli.verbose),
        Command::Cf => checkout(checkout_alias_arguments(false, true), jobs, cli.verbose),
        Command::Merge(arguments) => merge(arguments, jobs, cli.verbose),
        Command::Pull(arguments) => pull(arguments, jobs, cli.verbose),
        Command::Push(arguments) => push(arguments, jobs, cli.verbose),
        Command::Exec(arguments) => exec(arguments, jobs, cli.verbose),
        Command::Status => status(jobs),
        Command::Find(arguments) => find(arguments, jobs),
        Command::Info(arguments) => info(arguments, jobs),
        Command::Branch => branch(jobs),
        Command::List(arguments) => list(arguments, jobs),
        Command::Forget(arguments) => forget(arguments),
    }
}

fn checkout_alias_arguments(default: bool, feature: bool) -> CheckoutArgs {
    CheckoutArgs {
        create: false,
        branch: None,
        default,
        feature,
        from: None,
        remote: None,
    }
}

pub fn passthrough(args: Vec<OsString>, options: RuntimeOptions) -> Result<i32> {
    let jobs = settings::jobs(options.jobs)?;
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    let workspace = workspace::read(&root)?;
    let verbose = options.verbose || settings::passthrough_verbose()?;
    let results = map_ordered(&workspace.repositories, jobs, |repository| {
        let path = root.join(&repository.directory);
        if !git::is_repository(&path) {
            return RepositoryResult::skipped(repository, "repository is not materialized");
        }
        match git::run_os(&path, &args, false, jobs == 1) {
            Ok(output) => passthrough_result(repository, &args, output),
            Err(error) => RepositoryResult::failed(repository, error.to_string()),
        }
    })?;
    Ok(print_results(&results, verbose))
}

fn exec(arguments: ExecArgs, jobs: usize, verbose: bool) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    let workspace = workspace::read(&root)?;
    let mut selected = HashSet::new();

    for selector in &arguments.selectors {
        let matches = workspace
            .repositories
            .iter()
            .enumerate()
            .filter_map(|(index, repository)| {
                (repository.name == *selector || repository.directory == *selector).then_some(index)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [index] => {
                selected.insert(*index);
            }
            [] => bail!("unknown repository selector: {selector}"),
            _ => bail!("ambiguous repository selector: {selector}"),
        }
    }

    for pattern in &arguments.matches {
        let matches = workspace
            .repositories
            .iter()
            .enumerate()
            .filter_map(|(index, repository)| {
                wildcard_matches(pattern, &repository.name).then_some(index)
            })
            .collect::<Vec<_>>();
        if matches.is_empty() {
            bail!("repository pattern matched nothing: {pattern}");
        }
        selected.extend(matches);
    }

    let repositories = workspace
        .repositories
        .iter()
        .enumerate()
        .filter_map(|(index, repository)| selected.contains(&index).then_some(repository.clone()))
        .collect::<Vec<_>>();
    let results = map_ordered(&repositories, jobs, |repository| {
        let path = root.join(&repository.directory);
        if !git::is_repository(&path) {
            return RepositoryResult::failed(repository, "repository is not materialized");
        }
        match git::run_os(&path, &arguments.git_args, false, jobs == 1) {
            Ok(output) => passthrough_result(repository, &arguments.git_args, output),
            Err(error) => RepositoryResult::failed(repository, error.to_string()),
        }
    })?;
    Ok(print_selected_results(&results, verbose))
}

fn passthrough_result(
    repository: &RepositoryRecord,
    args: &[OsString],
    output: GitOutput,
) -> RepositoryResult {
    if is_nothing_to_commit(args, &output) {
        RepositoryResult::skipped_from_git(repository, "nothing to commit", output)
    } else {
        RepositoryResult::from_git(repository, output, "Git command completed", false)
    }
}

fn is_nothing_to_commit(args: &[OsString], output: &GitOutput) -> bool {
    if output.success || output.code != Some(1) || args.first().is_none_or(|arg| arg != "commit") {
        return false;
    }
    let message = format!("{}\n{}", output.stdout, output.stderr).to_ascii_lowercase();
    [
        "nothing to commit",
        "no changes added to commit",
        "nothing added to commit",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

fn clone_repository(arguments: CloneArgs) -> Result<i32> {
    let root = workspace::find_root_optional()?.unwrap_or(workspace::current_root()?);
    let _lock = WorkspaceLock::acquire(&root)?;
    let mut manifest = workspace::read_or_new(&root)?;
    let directory = arguments
        .directory
        .unwrap_or_else(|| PathBuf::from(default_clone_directory(&arguments.repository)));
    let directory_string = relative_string(&directory)?;
    validate_directory(&directory_string)?;
    if manifest
        .repositories
        .iter()
        .any(|repository| repository.directory == directory_string)
    {
        bail!("repository directory is already registered: {directory_string}");
    }

    let target = root.join(&directory);
    if target.exists() {
        bail!("clone destination already exists: {}", target.display());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    if let Some(depth) = arguments.depth
        && depth == 0
    {
        bail!("--depth must be at least 1");
    }
    if let Err(error) = git::clone_repository(
        &arguments.repository,
        &target,
        CloneOptions {
            remote_name: "origin",
            branch: arguments.branch.as_deref(),
            depth: arguments.depth,
            single_branch: arguments.single_branch,
            progress: None,
        },
    ) {
        eprintln!("clone failed: {error:#}");
        return Ok(1);
    }

    let info = git::inspect(&target)?;
    let name = unique_name(
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("repository"),
        &directory_string,
        &manifest.repositories,
    );
    let timestamp = now();
    manifest.repositories.push(RepositoryRecord {
        name: name.clone(),
        directory: directory_string,
        default_branch: info.default_branch,
        primary_remote: info.primary_remote,
        remotes: info.remotes,
        created_at: timestamp.clone(),
        synced_at: Some(timestamp),
    });
    workspace::write(&root, &mut manifest)?;
    println!(
        "registered {name} in {}",
        root.join(WORKSPACE_FILE).display()
    );
    Ok(0)
}

fn scan(arguments: ScanArgs, jobs: usize) -> Result<i32> {
    let root = workspace::current_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    let mut manifest = workspace::read_or_new(&root)?;
    let depth = settings::scan_depth(arguments.depth)?;
    let paths = git::discover(&root, depth)?;
    let known_directories: HashSet<String> = manifest
        .repositories
        .iter()
        .map(|repository| repository.directory.clone())
        .collect();
    let candidates: Vec<(PathBuf, String)> = paths
        .into_iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(&root).ok()?;
            let directory = relative_string(relative).ok()?;
            (!known_directories.contains(&directory)).then_some((path, directory))
        })
        .collect();

    let inspected = map_ordered(&candidates, jobs, |(path, directory)| {
        (path.clone(), directory.clone(), git::inspect(path))
    })?;
    let mut failures = 0;
    for (path, directory, inspection) in inspected {
        match inspection {
            Ok(info) => {
                let base_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("repository");
                let name = unique_name(base_name, &directory, &manifest.repositories);
                manifest.repositories.push(RepositoryRecord {
                    name: name.clone(),
                    directory: directory.clone(),
                    default_branch: info.default_branch,
                    primary_remote: info.primary_remote,
                    remotes: info.remotes,
                    created_at: now(),
                    synced_at: None,
                });
                println!("added {name} ({directory})");
            }
            Err(error) => {
                failures += 1;
                eprintln!("failed {}: {error:#}", path.display());
            }
        }
    }
    workspace::write(&root, &mut manifest)?;
    println!(
        "workspace: {} repositories, {} newly added, {} failed",
        manifest.repositories.len(),
        candidates.len().saturating_sub(failures),
        failures
    );
    Ok(if failures == 0 { 0 } else { 1 })
}

fn restore(jobs: usize, verbose: bool) -> Result<i32> {
    let root = workspace::current_root()?;
    if !root.join(WORKSPACE_FILE).is_file() {
        bail!("restore requires {} in {}", WORKSPACE_FILE, root.display());
    }
    let _lock = WorkspaceLock::acquire(&root)?;
    let mut manifest = workspace::read(&root)?;
    let records = manifest.repositories.clone();
    let progress = OperationProgress::new(&records);
    let results = map_ordered(&records, jobs, |repository| {
        let bar = progress.bar(repository);
        restore_one(&root, repository, jobs == 1, bar.as_ref())
    })?;
    progress.finish();
    for (record, outcome) in manifest.repositories.iter_mut().zip(&results) {
        if outcome.was_synced() {
            record.synced_at = Some(now());
        }
    }
    if results.iter().any(RepositoryResult::was_synced) {
        workspace::write(&root, &mut manifest)?;
    }
    Ok(crate::report::print_operation_summary(&results, verbose))
}

fn restore_one(
    root: &Path,
    repository: &RepositoryRecord,
    allow_stdin: bool,
    progress: Option<&ProgressBar>,
) -> RepositoryResult {
    set_operation_status(progress, "checking", 0);
    let target = root.join(&repository.directory);
    if target.exists() {
        if !git::is_repository(&target) {
            let result =
                RepositoryResult::failed(repository, "target exists but is not a Git repository");
            finish_operation(progress, &result);
            return result;
        }
        let result = match git::verify_declared_remotes(&target, repository)
            .and_then(|()| git::configure_declared_remotes(&target, repository, allow_stdin))
        {
            Ok(()) => RepositoryResult::skipped(repository, "existing repository verified"),
            Err(error) => RepositoryResult::failed(repository, error.to_string()),
        };
        finish_operation(progress, &result);
        return result;
    }

    let Some(primary) = repository
        .remotes
        .iter()
        .find(|remote| remote.name == repository.primary_remote)
    else {
        let result = RepositoryResult::failed(repository, "primary remote is not declared");
        finish_operation(progress, &result);
        return result;
    };
    if let Some(parent) = target.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        let result = RepositoryResult::failed(repository, error.to_string());
        finish_operation(progress, &result);
        return result;
    }
    set_operation_status(progress, "cloning", 0);
    let clone_progress = progress.cloned().map(|bar| {
        Arc::new(move |received: usize, total: usize| {
            let percentage = received.saturating_mul(100).checked_div(total).unwrap_or(0);
            bar.set_position(percentage as u64);
            bar.set_message(format!("receiving {received}/{total}"));
        }) as git::CloneProgress
    });
    match git::clone_repository(
        &primary.fetch_url,
        &target,
        CloneOptions {
            remote_name: &repository.primary_remote,
            branch: Some(&repository.default_branch),
            depth: None,
            single_branch: true,
            progress: clone_progress,
        },
    ) {
        Ok(_) => {}
        Err(error) => {
            let result = RepositoryResult::failed(repository, error.to_string());
            finish_operation(progress, &result);
            return result;
        }
    }
    set_operation_status(progress, "configuring", 98);
    if let Err(error) = git::configure_declared_remotes(&target, repository, allow_stdin) {
        let result = RepositoryResult::failed(repository, error.to_string());
        finish_operation(progress, &result);
        return result;
    }
    let result = RepositoryResult::success(repository, "restored default branch", true);
    finish_operation(progress, &result);
    result
}

struct OperationProgress {
    multi: Option<MultiProgress>,
    bars: Vec<(String, ProgressBar)>,
}

impl OperationProgress {
    fn new(repositories: &[RepositoryRecord]) -> Self {
        if !io::stderr().is_terminal() {
            return Self {
                multi: None,
                bars: Vec::new(),
            };
        }
        let multi = MultiProgress::new();
        let name_width = repositories
            .iter()
            .map(|repository| repository.name.chars().count())
            .max()
            .unwrap_or(10)
            .max("REPOSITORY".len());
        let style = ProgressStyle::with_template(&format!(
            "{{prefix:<{name_width}}}  [{{bar:28.cyan/blue}}] {{pos:>3}}%  {{msg}}"
        ))
        .expect("valid operation progress template")
        .progress_chars("━━╸");
        let bars = repositories
            .iter()
            .map(|repository| {
                let bar = multi.add(ProgressBar::new(100));
                bar.set_style(style.clone());
                bar.set_prefix(repository.name.clone());
                bar.set_message("queued");
                (repository.name.clone(), bar)
            })
            .collect();
        Self {
            multi: Some(multi),
            bars,
        }
    }

    fn bar(&self, repository: &RepositoryRecord) -> Option<ProgressBar> {
        self.bars
            .iter()
            .find(|(name, _)| name == &repository.name)
            .map(|(_, bar)| bar.clone())
    }

    fn finish(self) {
        if let Some(multi) = self.multi {
            let _ = multi.clear();
        }
    }
}

fn set_operation_status(progress: Option<&ProgressBar>, message: &'static str, position: u64) {
    if let Some(progress) = progress {
        progress.set_position(position);
        progress.set_message(message);
    }
}

fn finish_operation(progress: Option<&ProgressBar>, result: &RepositoryResult) {
    if let Some(progress) = progress {
        progress.set_position(100);
        progress.finish_with_message(result.progress_label());
    }
}

fn fetch(jobs: usize, verbose: bool) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    let mut manifest = workspace::read(&root)?;
    let records = manifest.repositories.clone();
    let progress = OperationProgress::new(&records);
    let results = map_ordered(&records, jobs, |repository| {
        let bar = progress.bar(repository);
        set_operation_status(bar.as_ref(), "checking", 0);
        let path = root.join(&repository.directory);
        if !git::is_repository(&path) {
            let result =
                RepositoryResult::failed(repository, "repository is not materialized; run restore");
            finish_operation(bar.as_ref(), &result);
            return result;
        }
        set_operation_status(bar.as_ref(), "configuring", 0);
        if let Err(error) = git::configure_declared_remotes(&path, repository, jobs == 1) {
            let result = RepositoryResult::failed(repository, error.to_string());
            finish_operation(bar.as_ref(), &result);
            return result;
        }
        set_operation_status(bar.as_ref(), "fetching", 0);
        let result = match git::fetch_all(&path, jobs == 1) {
            Ok(output) => {
                RepositoryResult::from_git(repository, output, "remote refs updated", true)
            }
            Err(error) => RepositoryResult::failed(repository, error.to_string()),
        };
        finish_operation(bar.as_ref(), &result);
        result
    })?;
    progress.finish();
    for (record, outcome) in manifest.repositories.iter_mut().zip(&results) {
        if outcome.was_synced() {
            record.synced_at = Some(now());
        }
    }
    if results.iter().any(RepositoryResult::was_synced) {
        workspace::write(&root, &mut manifest)?;
    }
    Ok(crate::report::print_operation_summary(&results, verbose))
}

fn sync(arguments: SyncArgs, jobs: usize, verbose: bool) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    let mut manifest = workspace::read(&root)?;
    let records = crate::selector::select(
        &manifest,
        &arguments.selectors,
        &arguments.matches,
        arguments.all,
    )?;
    run_sync(&root, &mut manifest, &records, jobs, verbose)
}

pub(crate) fn run_sync(
    root: &Path,
    manifest: &mut Workspace,
    records: &[RepositoryRecord],
    jobs: usize,
    verbose: bool,
) -> Result<i32> {
    let progress = OperationProgress::new(records);
    let results = map_ordered(records, jobs, |repository| {
        let bar = progress.bar(repository);
        set_operation_status(bar.as_ref(), "checking", 0);
        let restore_result = restore_one(root, repository, jobs == 1, None);
        if restore_result.is_failed() {
            finish_operation(bar.as_ref(), &restore_result);
            return restore_result;
        }
        let restored = restore_result.was_synced();
        set_operation_status(bar.as_ref(), "fetching", 0);
        let path = root.join(&repository.directory);
        let result = match git::fetch_all(&path, jobs == 1) {
            Ok(output) => RepositoryResult::from_git(
                repository,
                output,
                if restored {
                    "restored and remote refs updated"
                } else {
                    "remote refs updated"
                },
                true,
            ),
            Err(error) => RepositoryResult::failed(repository, error.to_string()),
        };
        finish_operation(bar.as_ref(), &result);
        result
    })?;
    progress.finish();

    let timestamp = now();
    let mut changed = false;
    for (selected, outcome) in records.iter().zip(&results) {
        if !outcome.was_synced() {
            continue;
        }
        if let Some(record) = manifest
            .repositories
            .iter_mut()
            .find(|record| record.name == selected.name)
        {
            record.synced_at = Some(timestamp.clone());
            changed = true;
        }
    }
    if changed {
        workspace::write(root, manifest)?;
    }
    Ok(crate::report::print_operation_summary(&results, verbose))
}

fn pull(arguments: SyncArgs, jobs: usize, verbose: bool) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    let mut manifest = workspace::read(&root)?;
    let records = crate::selector::select(
        &manifest,
        &arguments.selectors,
        &arguments.matches,
        arguments.all,
    )?;
    run_pull(&root, &mut manifest, &records, jobs, verbose)
}

pub(crate) fn run_pull(
    root: &Path,
    manifest: &mut Workspace,
    records: &[RepositoryRecord],
    jobs: usize,
    verbose: bool,
) -> Result<i32> {
    let results = map_ordered(records, jobs, |repository| {
        let path = root.join(&repository.directory);
        if !git::is_repository(&path) {
            return RepositoryResult::failed(
                repository,
                "repository is not materialized; run sync or restore",
            );
        }
        let status = match git::status_summary(&path) {
            Ok(status) => status,
            Err(error) => return RepositoryResult::failed(repository, error.to_string()),
        };
        if status.branch.starts_with("(detached:") || status.branch == "(unborn)" {
            return RepositoryResult::failed(repository, "current HEAD is not a local branch");
        }
        if status.changes.total() != 0 {
            return RepositoryResult::failed(repository, "working tree is not clean");
        }
        if matches!(status.upstream, UpstreamSummary::None) {
            return RepositoryResult::failed(repository, "current branch has no upstream");
        }
        match git::run(&path, ["pull", "--ff-only"], true, false) {
            Ok(output) => RepositoryResult::from_git(
                repository,
                output,
                format!("branch {} updated with fast-forward only", status.branch),
                true,
            ),
            Err(error) => RepositoryResult::failed(repository, error.to_string()),
        }
    })?;

    let timestamp = now();
    let mut changed = false;
    for (selected, outcome) in records.iter().zip(&results) {
        if !outcome.was_synced() {
            continue;
        }
        if let Some(record) = manifest
            .repositories
            .iter_mut()
            .find(|record| record.name == selected.name)
        {
            record.synced_at = Some(timestamp.clone());
            changed = true;
        }
    }
    if changed {
        workspace::write(root, manifest)?;
    }
    Ok(crate::report::print_operation_summary(&results, verbose))
}

fn push(arguments: PushArgs, jobs: usize, verbose: bool) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    let manifest = workspace::read(&root)?;
    let records = crate::selector::select(
        &manifest,
        &arguments.selection.selectors,
        &arguments.selection.matches,
        arguments.selection.all,
    )?;
    let results = map_ordered(&records, jobs, |repository| {
        let path = root.join(&repository.directory);
        if !git::is_repository(&path) {
            return RepositoryResult::failed(
                repository,
                "repository is not materialized; run sync or restore",
            );
        }
        let status = match git::status_summary(&path) {
            Ok(status) => status,
            Err(error) => return RepositoryResult::failed(repository, error.to_string()),
        };
        if status.branch.starts_with("(detached:") || status.branch == "(unborn)" {
            return RepositoryResult::failed(repository, "current HEAD is not a local branch");
        }
        let upstream_target = match git::upstream_push_target(&path, &status.branch) {
            Ok(target) => target,
            Err(error) => return RepositoryResult::failed(repository, error.to_string()),
        };

        match status.upstream {
            UpstreamSummary::None if upstream_target.is_some() => {
                let (remote, merge_ref) = upstream_target.as_ref().expect("target checked above");
                push_existing_upstream(
                    repository,
                    &path,
                    &status.branch,
                    remote,
                    merge_ref,
                    arguments.dry_run,
                    jobs == 1,
                )
            }
            UpstreamSummary::None if !arguments.set_upstream => {
                RepositoryResult::skipped(repository, "current branch has no upstream")
            }
            UpstreamSummary::None => {
                let remote = arguments
                    .remote
                    .as_deref()
                    .unwrap_or(&repository.primary_remote);
                let mut git_arguments = vec!["push"];
                if arguments.dry_run {
                    git_arguments.push("--dry-run");
                }
                git_arguments.extend(["--set-upstream", remote, "HEAD"]);
                match git::run(&path, git_arguments, true, jobs == 1) {
                    Ok(output) => RepositoryResult::from_git(
                        repository,
                        output,
                        if arguments.dry_run {
                            format!(
                                "would push {} to {remote} and configure upstream",
                                status.branch
                            )
                        } else {
                            format!(
                                "pushed {} to {remote} and configured upstream",
                                status.branch
                            )
                        },
                        false,
                    ),
                    Err(error) => RepositoryResult::failed(repository, error.to_string()),
                }
            }
            UpstreamSummary::UpToDate => RepositoryResult::skipped(repository, "nothing to push"),
            UpstreamSummary::Behind(count) => RepositoryResult::skipped(
                repository,
                format!("branch is behind upstream by {count} commit(s)"),
            ),
            UpstreamSummary::Diverged { ahead, behind } => RepositoryResult::failed(
                repository,
                format!("branch has diverged from upstream: ahead {ahead}, behind {behind}"),
            ),
            UpstreamSummary::Ahead(_) => {
                let Some((remote, merge_ref)) = upstream_target.as_ref() else {
                    return RepositoryResult::failed(
                        repository,
                        "current branch has no configured upstream target",
                    );
                };
                push_existing_upstream(
                    repository,
                    &path,
                    &status.branch,
                    remote,
                    merge_ref,
                    arguments.dry_run,
                    jobs == 1,
                )
            }
        }
    })?;
    Ok(print_push_summary(&results, verbose))
}

fn push_existing_upstream(
    repository: &RepositoryRecord,
    path: &Path,
    branch: &str,
    remote: &str,
    merge_ref: &str,
    dry_run: bool,
    allow_stdin: bool,
) -> RepositoryResult {
    let refspec = format!("HEAD:{merge_ref}");
    let mut git_arguments = vec!["push"];
    if dry_run {
        git_arguments.push("--dry-run");
    }
    git_arguments.extend([remote, refspec.as_str()]);
    match git::run(path, git_arguments, true, allow_stdin) {
        Ok(output) => RepositoryResult::from_git(
            repository,
            output,
            format!("pushed {branch}{}", if dry_run { " (dry run)" } else { "" }),
            false,
        ),
        Err(error) => RepositoryResult::failed(repository, error.to_string()),
    }
}

fn checkout(arguments: CheckoutArgs, jobs: usize, _verbose: bool) -> Result<i32> {
    let current_feature_branch = settings::current_feature_branch()?;
    let feature_branch = if arguments.feature {
        match current_feature_branch.as_deref() {
            Some(branch) => Some(branch),
            None => {
                println!("CURRENT_FEATURE_BRANCH is not set; nothing to checkout");
                return Ok(0);
            }
        }
    } else {
        None
    };
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    let manifest = workspace::read(&root)?;
    if arguments.create && arguments.from.is_none() && arguments.remote.is_some() {
        bail!("--remote requires --from when used with checkout -b");
    }
    let remote = if arguments.create && arguments.from.is_none() {
        None
    } else {
        settings::checkout_remote(arguments.remote)
    };
    let results = map_ordered(&manifest.repositories, jobs, |repository| {
        let path = root.join(&repository.directory);
        if !git::is_repository(&path) {
            return RepositoryResult::failed(
                repository,
                "repository is not materialized; run restore",
            );
        }
        let branch = arguments
            .branch
            .as_deref()
            .or(feature_branch)
            .unwrap_or(&repository.default_branch);
        if arguments.create {
            return match git::create_and_checkout_branch(
                &path,
                branch,
                arguments.from.as_deref(),
                remote.as_deref(),
            ) {
                Ok(()) => {
                    RepositoryResult::success(repository, format!("created branch {branch}"), false)
                }
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            };
        }
        let selected_remote = if arguments.default {
            Some(repository.primary_remote.as_str())
        } else {
            remote.as_deref()
        };
        let target = match git::checkout_target(&path, branch, selected_remote) {
            Ok(target) => target,
            Err(error) => return RepositoryResult::failed(repository, error.to_string()),
        };
        match target {
            CheckoutTarget::Missing => {
                RepositoryResult::skipped(repository, format!("branch {branch} does not exist"))
            }
            CheckoutTarget::Ambiguous(matches) => RepositoryResult::failed(
                repository,
                format!("branch is ambiguous: {}", matches.join(", ")),
            ),
            CheckoutTarget::Local => match git::checkout_local(&path, branch) {
                Ok(()) => RepositoryResult::success(repository, "checked out local branch", false),
                Err(error) => RepositoryResult::failed(repository, error.to_string()),
            },
            CheckoutTarget::Remote(remote_branch) => {
                match git::checkout_remote(&path, branch, &remote_branch) {
                    Ok(()) => RepositoryResult::success(
                        repository,
                        format!("created tracking branch from {remote_branch}"),
                        false,
                    ),
                    Err(error) => RepositoryResult::failed(repository, error.to_string()),
                }
            }
        }
    })?;
    let branches = manifest
        .repositories
        .iter()
        .map(|repository| {
            let path = root.join(&repository.directory);
            if !path.exists() {
                "(missing)".to_owned()
            } else if !git::is_repository(&path) {
                "(not-git)".to_owned()
            } else {
                git::current_branch_summary(&path).unwrap_or_else(|_| "(unknown)".to_owned())
            }
        })
        .collect::<Vec<_>>();
    let default_branches = manifest
        .repositories
        .iter()
        .map(|repository| repository.default_branch.clone())
        .collect::<Vec<_>>();
    Ok(print_checkout_summary(
        &results,
        &branches,
        &default_branches,
        current_feature_branch.as_deref(),
    ))
}

fn merge(arguments: MergeArgs, jobs: usize, verbose: bool) -> Result<i32> {
    let feature_branch = if arguments.feature {
        match settings::current_feature_branch()? {
            Some(branch) => Some(branch),
            None => {
                println!("CURRENT_FEATURE_BRANCH is not set; nothing to merge");
                return Ok(0);
            }
        }
    } else {
        None
    };
    let branch = arguments
        .branch
        .as_deref()
        .or(feature_branch.as_deref())
        .expect("clap requires a branch or --feature");
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    let mut manifest = workspace::read(&root)?;
    let update_current = settings::merge_update_current(if arguments.update_current {
        Some(true)
    } else if arguments.no_update_current {
        Some(false)
    } else {
        None
    })?;
    let results = map_ordered(&manifest.repositories, jobs, |repository| {
        let path = root.join(&repository.directory);
        if !git::is_repository(&path) {
            return RepositoryResult::failed(
                repository,
                "repository is not materialized; run restore",
            );
        }
        let status = match git::status_summary(&path) {
            Ok(status) => status,
            Err(error) => return RepositoryResult::failed(repository, error.to_string()),
        };
        if status.branch.starts_with("(detached:") || status.branch == "(unborn)" {
            return RepositoryResult::failed(repository, "current HEAD is not a local branch");
        }
        if status.changes.total() != 0 {
            return RepositoryResult::failed(repository, "working tree is not clean");
        }
        if status.branch == branch {
            return RepositoryResult::skipped(repository, "source is the current branch");
        }

        if update_current {
            let output = match git::run(&path, ["pull", "--ff-only"], true, jobs == 1) {
                Ok(output) => output,
                Err(error) => return RepositoryResult::failed(repository, error.to_string()),
            };
            if !output.success {
                return RepositoryResult::from_git(
                    repository,
                    output,
                    "current branch updated",
                    false,
                );
            }
        }

        let source = match git::checkout_target(&path, branch, arguments.remote.as_deref()) {
            Ok(CheckoutTarget::Local) => branch.to_owned(),
            Ok(CheckoutTarget::Remote(remote_branch)) => remote_branch,
            Ok(CheckoutTarget::Missing) => {
                return RepositoryResult::skipped(
                    repository,
                    format!("branch {branch} does not exist"),
                );
            }
            Ok(CheckoutTarget::Ambiguous(matches)) => {
                return RepositoryResult::failed(
                    repository,
                    format!("branch is ambiguous: {}", matches.join(", ")),
                );
            }
            Err(error) => return RepositoryResult::failed(repository, error.to_string()),
        };
        match git::run(
            &path,
            ["merge", "--no-edit", source.as_str()],
            true,
            jobs == 1,
        ) {
            Ok(output) => RepositoryResult::from_git(
                repository,
                output,
                format!("merged {source} into {}", status.branch),
                update_current,
            ),
            Err(error) => RepositoryResult::failed(repository, error.to_string()),
        }
    })?;
    for (record, outcome) in manifest.repositories.iter_mut().zip(&results) {
        if outcome.was_synced() {
            record.synced_at = Some(now());
        }
    }
    if results.iter().any(RepositoryResult::was_synced) {
        workspace::write(&root, &mut manifest)?;
    }
    Ok(print_selected_results(&results, verbose))
}

fn list(arguments: ListArgs, jobs: usize) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let repositories: Vec<ListRepository<'_>> = manifest
        .repositories
        .iter()
        .map(|repository| {
            let path = root.join(&repository.directory);
            let materialized = git::is_repository(&path);
            ListRepository {
                name: &repository.name,
                directory: &repository.directory,
                default_branch: &repository.default_branch,
                current_branch: materialized
                    .then(|| git::current_branch_summary(&path).ok())
                    .flatten(),
                materialized,
                synced_at: repository.synced_at.as_deref(),
            }
        })
        .collect();
    if arguments.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&ListOutput {
                workspace: root.to_string_lossy().into_owned(),
                jobs,
                repositories,
            })?
        );
    } else {
        let feature_branch = settings::current_feature_branch()?;
        let rows: Vec<Vec<String>> = repositories
            .into_iter()
            .map(|repository| {
                let state = if repository.materialized {
                    repository
                        .current_branch
                        .clone()
                        .unwrap_or_else(|| color::yellow("unknown"))
                } else {
                    color::red("missing")
                };
                vec![
                    repository.name.to_owned(),
                    color::branch(&state, repository.default_branch, feature_branch.as_deref()),
                    color::blue(repository.default_branch),
                ]
            })
            .collect();
        print!("{}", table::render(&["NAME", "CURRENT", "DEFAULT"], &rows));
        println!();
        println!("{} repositories", manifest.repositories.len());
    }
    Ok(0)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkspaceStatusKind {
    Clean,
    Dirty,
    Conflict,
    Missing,
    NotGit,
    Error,
}

impl WorkspaceStatusKind {
    fn label(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Dirty => "dirty",
            Self::Conflict => "conflict",
            Self::Missing => "missing",
            Self::NotGit => "not-git",
            Self::Error => "error",
        }
    }

    fn unavailable(self) -> bool {
        matches!(self, Self::Missing | Self::NotGit | Self::Error)
    }

    fn colored_label(self) -> String {
        match self {
            Self::Clean => color::green(self.label()),
            Self::Dirty => color::yellow(self.label()),
            Self::Conflict | Self::Missing | Self::NotGit | Self::Error => color::red(self.label()),
        }
    }

    fn colored_count(self, count: usize) -> String {
        match self {
            Self::Clean => color::green(count),
            Self::Dirty => color::yellow(count),
            Self::Conflict | Self::Missing | Self::NotGit | Self::Error => color::red(count),
        }
    }
}

struct WorkspaceStatusRow {
    repository: String,
    branch: String,
    default_branch: String,
    kind: WorkspaceStatusKind,
    changes: String,
    upstream: String,
    changed_paths: usize,
}

fn status(jobs: usize) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let feature_branch = settings::current_feature_branch()?;
    let statuses = map_ordered(&manifest.repositories, jobs, |repository| {
        let path = root.join(&repository.directory);
        if !path.exists() {
            return WorkspaceStatusRow {
                repository: repository.name.clone(),
                branch: "-".to_owned(),
                default_branch: repository.default_branch.clone(),
                kind: WorkspaceStatusKind::Missing,
                changes: "-".to_owned(),
                upstream: "-".to_owned(),
                changed_paths: 0,
            };
        }
        if !git::is_repository(&path) {
            return WorkspaceStatusRow {
                repository: repository.name.clone(),
                branch: "-".to_owned(),
                default_branch: repository.default_branch.clone(),
                kind: WorkspaceStatusKind::NotGit,
                changes: "-".to_owned(),
                upstream: "-".to_owned(),
                changed_paths: 0,
            };
        }
        match git::status_summary(&path) {
            Ok(status) => {
                let kind = if status.changes.conflicted > 0 {
                    WorkspaceStatusKind::Conflict
                } else if status.changes.total() > 0 {
                    WorkspaceStatusKind::Dirty
                } else {
                    WorkspaceStatusKind::Clean
                };
                WorkspaceStatusRow {
                    repository: repository.name.clone(),
                    branch: status.branch,
                    default_branch: repository.default_branch.clone(),
                    kind,
                    changes: status.changes.compact(),
                    upstream: status.upstream.label(),
                    changed_paths: status.changes.total(),
                }
            }
            Err(_) => WorkspaceStatusRow {
                repository: repository.name.clone(),
                branch: git::current_branch_summary(&path)
                    .unwrap_or_else(|_| "(unknown)".to_owned()),
                default_branch: repository.default_branch.clone(),
                kind: WorkspaceStatusKind::Error,
                changes: "-".to_owned(),
                upstream: "-".to_owned(),
                changed_paths: 0,
            },
        }
    })?;
    let rows = statuses
        .iter()
        .map(|status| {
            vec![
                status.repository.clone(),
                status.kind.colored_label(),
                status.changes.clone(),
                status.upstream.clone(),
                color::branch(
                    &status.branch,
                    &status.default_branch,
                    feature_branch.as_deref(),
                ),
            ]
        })
        .collect::<Vec<_>>();
    print!(
        "{}",
        table::render(
            &["REPOSITORY", "STATE", "CHANGES", "UPSTREAM", "BRANCH"],
            &rows
        )
    );

    let state_counts = [
        (WorkspaceStatusKind::Clean, "clean"),
        (WorkspaceStatusKind::Dirty, "dirty"),
        (WorkspaceStatusKind::Conflict, "conflict"),
        (WorkspaceStatusKind::Missing, "missing"),
        (WorkspaceStatusKind::NotGit, "not-git"),
        (WorkspaceStatusKind::Error, "error"),
    ]
    .into_iter()
    .filter_map(|(kind, label)| {
        let count = statuses.iter().filter(|status| status.kind == kind).count();
        (count > 0).then(|| format!("{} {label}", kind.colored_count(count)))
    })
    .collect::<Vec<_>>()
    .join(", ");
    let changed_paths = statuses
        .iter()
        .map(|status| status.changed_paths)
        .sum::<usize>();
    let state_summary = if state_counts.is_empty() {
        String::new()
    } else {
        format!(", {state_counts}")
    };
    println!();
    let changed_paths = if changed_paths > 0 {
        color::yellow(changed_paths)
    } else {
        changed_paths.to_string()
    };
    println!(
        "summary: {} repositories{state_summary}; {changed_paths} changed paths",
        statuses.len()
    );
    Ok(i32::from(
        statuses.iter().any(|status| status.kind.unavailable()),
    ))
}

fn find(arguments: FindArgs, jobs: usize) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let repositories = manifest
        .repositories
        .iter()
        .filter(|repository| {
            arguments
                .repo
                .as_deref()
                .is_none_or(|pattern| wildcard_matches(pattern, &repository.name))
        })
        .cloned()
        .collect::<Vec<_>>();
    let include_local = !arguments.remote;
    let include_remote = !arguments.local;
    let scans = map_ordered(&repositories, jobs, |repository| {
        let path = root.join(&repository.directory);
        if !git::is_repository(&path) {
            return (repository.clone(), None);
        }
        let branches = git::branches(&path, include_local, include_remote)
            .ok()
            .map(|branches| {
                branches
                    .into_iter()
                    .filter(|branch| wildcard_matches(&arguments.pattern, &branch.name))
                    .collect::<Vec<_>>()
            });
        (repository.clone(), branches)
    })?;

    let unavailable = scans
        .iter()
        .filter(|(_, branches)| branches.is_none())
        .count();
    let mut matches = Vec::new();
    for (repository, branches) in scans {
        for branch in branches.unwrap_or_default() {
            matches.push(FindBranch {
                repository: repository.name.clone(),
                directory: repository.directory.clone(),
                name: branch.name,
                default_branch: repository.default_branch.clone(),
                kind: branch.kind.label(),
                remote: branch.remote,
                is_current: branch.is_current,
                commit: branch.commit,
                commit_short: branch.commit_short,
                commit_time: branch.commit_time,
            });
        }
    }
    let repositories_with_matches = matches
        .iter()
        .map(|branch| branch.repository.as_str())
        .collect::<HashSet<_>>()
        .len();
    let local = matches
        .iter()
        .filter(|branch| branch.kind == BranchKind::Local.label())
        .count();
    let remote = matches.len() - local;

    if arguments.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&FindOutput {
                pattern: &arguments.pattern,
                repository_pattern: arguments.repo.as_deref(),
                repositories_scanned: repositories.len(),
                unavailable,
                branches: matches,
            })?
        );
    } else {
        let feature_branch = settings::current_feature_branch()?;
        let rows = matches
            .iter()
            .map(|branch| {
                vec![
                    branch.repository.clone(),
                    branch.kind.to_owned(),
                    branch.remote.clone().unwrap_or_else(|| "-".to_owned()),
                    if branch.is_current {
                        color::green("yes")
                    } else {
                        "no".to_owned()
                    },
                    color::branch(
                        &branch.name,
                        &branch.default_branch,
                        feature_branch.as_deref(),
                    ),
                ]
            })
            .collect::<Vec<_>>();
        print!(
            "{}",
            table::render(
                &["REPOSITORY", "KIND", "REMOTE", "CURRENT", "BRANCH"],
                &rows
            )
        );
        println!();
        let unavailable_summary = if unavailable == 0 {
            String::new()
        } else {
            format!("; {} unavailable", color::red(unavailable))
        };
        println!(
            "summary: {} {} in {repositories_with_matches} {}; {local} local, {remote} remote{unavailable_summary}",
            matches.len(),
            if matches.len() == 1 {
                "branch"
            } else {
                "branches"
            },
            if repositories_with_matches == 1 {
                "repository"
            } else {
                "repositories"
            },
        );
    }
    Ok(i32::from(unavailable > 0))
}

fn info(arguments: InfoArgs, jobs: usize) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let current_feature_branch = settings::current_feature_branch()?;
    let Some(selector) = arguments.repository.as_deref() else {
        let runtime = map_ordered(&manifest.repositories, jobs, |repository| {
            git::repository_runtime_info(&root.join(&repository.directory))
        })?;
        let count = |state| {
            runtime
                .iter()
                .filter(|repository| repository.state == state)
                .count()
        };
        let output = WorkspaceInfoOutput {
            root: root.to_string_lossy().into_owned(),
            manifest: root.join(WORKSPACE_FILE).to_string_lossy().into_owned(),
            version: manifest.version,
            created_at: manifest.created_at.clone(),
            updated_at: manifest.updated_at.clone(),
            current_feature_branch,
            jobs,
            repositories: WorkspaceRepositoryCounts {
                total: manifest.repositories.len(),
                materialized: count(RepositoryRuntimeState::Available),
                missing: count(RepositoryRuntimeState::Missing),
                not_git: count(RepositoryRuntimeState::NotGit),
                bare: count(RepositoryRuntimeState::Bare),
                error: count(RepositoryRuntimeState::Error),
            },
        };
        if arguments.json {
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            let mut rows = vec![
                vec!["ROOT".to_owned(), output.root.clone()],
                vec!["MANIFEST".to_owned(), output.manifest.clone()],
                vec!["VERSION".to_owned(), output.version.to_string()],
                vec!["CREATED".to_owned(), output.created_at.clone()],
                vec!["UPDATED".to_owned(), output.updated_at.clone()],
            ];
            if let Some(branch) = &output.current_feature_branch {
                rows.push(vec!["FEATURE BRANCH".to_owned(), color::magenta(branch)]);
            }
            rows.extend([
                vec![
                    "REPOSITORIES".to_owned(),
                    output.repositories.total.to_string(),
                ],
                vec![
                    "MATERIALIZED".to_owned(),
                    color::green(output.repositories.materialized),
                ],
                vec![
                    "MISSING".to_owned(),
                    color::red(output.repositories.missing),
                ],
                vec![
                    "NOT-GIT".to_owned(),
                    color::red(output.repositories.not_git),
                ],
                vec!["BARE".to_owned(), color::red(output.repositories.bare)],
                vec!["ERROR".to_owned(), color::red(output.repositories.error)],
                vec!["JOBS".to_owned(), output.jobs.to_string()],
            ]);
            print!("{}", table::render(&["FIELD", "VALUE"], &rows));
        }
        return Ok(0);
    };

    let matches = manifest
        .repositories
        .iter()
        .filter(|repository| repository.name == selector || repository.directory == selector)
        .collect::<Vec<_>>();
    let repository = match matches.as_slice() {
        [repository] => *repository,
        [] => bail!("unknown repository selector: {selector}"),
        _ => bail!("ambiguous repository selector: {selector}"),
    };
    let path = root.join(&repository.directory);
    let runtime = git::repository_runtime_info(&path);
    let remotes = repository
        .remotes
        .iter()
        .map(|remote| InfoRemote {
            name: remote.name.clone(),
            primary: remote.name == repository.primary_remote,
            fetch_url: git::display_remote_url(&remote.fetch_url),
            push_url: remote.push_url.as_deref().map(git::display_remote_url),
        })
        .collect::<Vec<_>>();
    let output = RepositoryInfoOutput {
        name: repository.name.clone(),
        directory: repository.directory.clone(),
        path: path.to_string_lossy().into_owned(),
        state: runtime.state.label(),
        current_branch: runtime.current_branch.clone(),
        default_branch: repository.default_branch.clone(),
        head: runtime.head.clone(),
        primary_remote: repository.primary_remote.clone(),
        local_branches: runtime.local_branches,
        remote_branches: runtime.remote_branches,
        created_at: repository.created_at.clone(),
        synced_at: repository.synced_at.clone(),
        remotes,
    };
    if arguments.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_repository_info(&output, current_feature_branch.as_deref());
    }
    Ok(i32::from(
        runtime.state != RepositoryRuntimeState::Available,
    ))
}

fn print_repository_info(output: &RepositoryInfoOutput, feature_branch: Option<&str>) {
    let branches = match (output.local_branches, output.remote_branches) {
        (Some(local), Some(remote)) => format!("{local} local, {remote} remote"),
        _ => "-".to_owned(),
    };
    let rows = vec![
        vec!["NAME".to_owned(), output.name.clone()],
        vec!["DIRECTORY".to_owned(), output.directory.clone()],
        vec!["PATH".to_owned(), output.path.clone()],
        vec!["STATE".to_owned(), color_runtime_state(output.state)],
        vec![
            "CURRENT BRANCH".to_owned(),
            output.current_branch.as_ref().map_or_else(
                || "-".to_owned(),
                |branch| color::branch(branch, &output.default_branch, feature_branch),
            ),
        ],
        vec![
            "DEFAULT BRANCH".to_owned(),
            color::blue(&output.default_branch),
        ],
        vec![
            "HEAD".to_owned(),
            output.head.clone().unwrap_or_else(|| "-".to_owned()),
        ],
        vec!["PRIMARY REMOTE".to_owned(), output.primary_remote.clone()],
        vec!["BRANCHES".to_owned(), branches],
        vec!["CREATED".to_owned(), output.created_at.clone()],
        vec![
            "LAST SYNC".to_owned(),
            output.synced_at.clone().unwrap_or_else(|| "-".to_owned()),
        ],
    ];
    print!("{}", table::render(&["FIELD", "VALUE"], &rows));
    let remote_rows = output
        .remotes
        .iter()
        .map(|remote| {
            vec![
                remote.name.clone(),
                if remote.primary { "yes" } else { "no" }.to_owned(),
                remote.fetch_url.clone(),
            ]
        })
        .collect::<Vec<_>>();
    if !remote_rows.is_empty() {
        println!();
        print!(
            "{}",
            table::render(&["REMOTE", "PRIMARY", "FETCH URL"], &remote_rows)
        );
    }
}

fn color_runtime_state(state: &str) -> String {
    match state {
        "available" => color::green(state),
        "missing" | "not-git" | "bare" | "error" => color::red(state),
        _ => color::yellow(state),
    }
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let value = value.chars().collect::<Vec<_>>();
    let mut previous = vec![false; value.len() + 1];
    previous[0] = true;
    for token in pattern {
        let mut current = vec![false; value.len() + 1];
        if token == '*' {
            current[0] = previous[0];
            for index in 1..=value.len() {
                current[index] = previous[index] || current[index - 1];
            }
        } else {
            for index in 1..=value.len() {
                current[index] = previous[index - 1] && value[index - 1] == token;
            }
        }
        previous = current;
    }
    previous[value.len()]
}

fn branch(jobs: usize) -> Result<i32> {
    let root = workspace::find_root()?;
    let manifest = workspace::read(&root)?;
    let feature_branch = settings::current_feature_branch()?;
    let states = map_ordered(&manifest.repositories, jobs, |repository| {
        let path = root.join(&repository.directory);
        let state = if !path.exists() {
            color::red("(missing)")
        } else if !git::is_repository(&path) {
            color::red("(not-git)")
        } else {
            git::current_branch_summary(&path).unwrap_or_else(|_| color::yellow("(unknown)"))
        };
        (
            repository.name.clone(),
            color::branch(
                &state,
                &repository.default_branch,
                feature_branch.as_deref(),
            ),
        )
    })?;
    let rows: Vec<Vec<String>> = states
        .into_iter()
        .map(|(name, state)| vec![name, state])
        .collect();
    print!("{}", table::render(&["REPOSITORY", "BRANCH"], &rows));
    Ok(0)
}

fn forget(arguments: ForgetArgs) -> Result<i32> {
    let root = workspace::find_root()?;
    let _lock = WorkspaceLock::acquire(&root)?;
    let mut manifest = workspace::read(&root)?;
    let mut indexes = HashSet::new();
    for selector in &arguments.selectors {
        let matches: Vec<usize> = manifest
            .repositories
            .iter()
            .enumerate()
            .filter_map(|(index, repository)| {
                (repository.name == *selector || repository.directory == *selector).then_some(index)
            })
            .collect();
        match matches.as_slice() {
            [index] => {
                indexes.insert(*index);
            }
            [] => bail!("unknown repository selector: {selector}"),
            _ => bail!("ambiguous repository selector: {selector}"),
        }
    }
    let removed: Vec<String> = manifest
        .repositories
        .iter()
        .enumerate()
        .filter(|(index, _)| indexes.contains(index))
        .map(|(_, repository)| repository.name.clone())
        .collect();
    manifest.repositories = manifest
        .repositories
        .into_iter()
        .enumerate()
        .filter_map(|(index, repository)| (!indexes.contains(&index)).then_some(repository))
        .collect();
    workspace::write(&root, &mut manifest)?;
    for name in removed {
        println!("forgot {name}; repository directory was not deleted");
    }
    Ok(0)
}

fn default_clone_directory(repository: &str) -> String {
    let trimmed = repository.trim_end_matches('/').trim_end_matches(".git");
    trimmed
        .rsplit(['/', ':'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("repository")
        .to_owned()
}

fn relative_string(path: &Path) -> Result<String> {
    if path.is_absolute() {
        bail!("directory must be relative to the workspace");
    }
    let parts: Vec<String> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    Ok(parts.join("/"))
}

fn unique_name(base: &str, directory: &str, repositories: &[RepositoryRecord]) -> String {
    let used: HashSet<&str> = repositories
        .iter()
        .map(|repository| repository.name.as_str())
        .collect();
    if !used.contains(base) {
        return base.to_owned();
    }
    let candidate = directory.replace('/', "-");
    if !used.contains(candidate.as_str()) {
        return candidate;
    }
    for suffix in 2.. {
        let candidate = format!("{}-{suffix}", directory.replace('/', "-"));
        if !used.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!()
}

#[derive(Serialize)]
struct ListOutput<'a> {
    workspace: String,
    jobs: usize,
    repositories: Vec<ListRepository<'a>>,
}

#[derive(Serialize)]
struct ListRepository<'a> {
    name: &'a str,
    directory: &'a str,
    default_branch: &'a str,
    current_branch: Option<String>,
    materialized: bool,
    synced_at: Option<&'a str>,
}

#[derive(Serialize)]
struct FindOutput<'a> {
    pattern: &'a str,
    repository_pattern: Option<&'a str>,
    repositories_scanned: usize,
    unavailable: usize,
    branches: Vec<FindBranch>,
}

#[derive(Serialize)]
struct FindBranch {
    repository: String,
    directory: String,
    name: String,
    #[serde(skip)]
    default_branch: String,
    kind: &'static str,
    remote: Option<String>,
    is_current: bool,
    commit: String,
    commit_short: String,
    commit_time: i64,
}

#[derive(Serialize)]
struct WorkspaceInfoOutput {
    root: String,
    manifest: String,
    version: u32,
    created_at: String,
    updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_feature_branch: Option<String>,
    jobs: usize,
    repositories: WorkspaceRepositoryCounts,
}

#[derive(Serialize)]
struct WorkspaceRepositoryCounts {
    total: usize,
    materialized: usize,
    missing: usize,
    not_git: usize,
    bare: usize,
    error: usize,
}

#[derive(Serialize)]
struct RepositoryInfoOutput {
    name: String,
    directory: String,
    path: String,
    state: &'static str,
    current_branch: Option<String>,
    default_branch: String,
    head: Option<String>,
    primary_remote: String,
    local_branches: Option<usize>,
    remote_branches: Option<usize>,
    created_at: String,
    synced_at: Option<String>,
    remotes: Vec<InfoRemote>,
}

#[derive(Serialize)]
struct InfoRemote {
    name: String,
    primary: bool,
    fetch_url: String,
    push_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::wildcard_matches;

    #[test]
    fn wildcard_matches_zero_or_more_unicode_characters() {
        assert!(wildcard_matches("main", "main"));
        assert!(!wildcard_matches("main", "main-old"));
        assert!(wildcard_matches("feature/*", "feature/login"));
        assert!(wildcard_matches("*登录*", "feature/登录-v2"));
        assert!(wildcard_matches("release/*/hotfix", "release/1.0/hotfix"));
        assert!(!wildcard_matches("release/*/hotfix", "release/hotfix"));
        assert!(wildcard_matches("**main**", "main"));
    }
}
