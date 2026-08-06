use std::path::Path;

use anyhow::{Context, Result, bail};
use git2::Repository;
use url::Url;

use crate::model::RepositoryRecord;

use super::execution::run_with_options;
use super::types::{GitExecutionOptions, GitOutput};

/// fetch 所有远端并清理已删除的 remote-tracking 引用。
#[allow(dead_code)] // Retained as an internal compatibility wrapper for the previous call shape.
pub fn fetch_all(path: &Path, allow_stdin: bool) -> Result<GitOutput> {
    fetch_all_with_options(path, GitExecutionOptions::legacy(allow_stdin))
}

/// fetch 所有远端并使用显式的自动化执行选项。
pub fn fetch_all_with_options(path: &Path, options: GitExecutionOptions) -> Result<GitOutput> {
    run_with_options(path, ["fetch", "--all", "--prune"], true, options)
}

/// 解析当前分支配置的 push 远端和目标引用。
pub fn upstream_push_target(path: &Path, branch: &str) -> Result<Option<(String, String)>> {
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    let config = repository
        .config()
        .context("failed to read Git configuration")?;
    let remote_key = format!("branch.{branch}.remote");
    let merge_key = format!("branch.{branch}.merge");
    let remote = match config.get_string(&remote_key) {
        Ok(remote) => remote,
        Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if remote == "." {
        bail!("current branch upstream is not a remote branch");
    }
    let merge_ref = config
        .get_string(&merge_key)
        .with_context(|| format!("current branch has no configured upstream ref: {branch}"))?;
    if !merge_ref.starts_with("refs/heads/") {
        bail!("current branch upstream is not a remote branch: {merge_ref}");
    }
    Ok(Some((remote, merge_ref)))
}

/// 只验证本地远端配置是否与清单一致，不进行修改。
pub fn verify_declared_remotes(path: &Path, repository: &RepositoryRecord) -> Result<()> {
    for remote in &repository.remotes {
        match remote_url(path, &remote.name)? {
            Some(actual) if actual == remote.fetch_url => {}
            Some(actual) => bail!(
                "remote {} URL mismatch: expected {}, found {}",
                remote.name,
                remote.fetch_url,
                actual
            ),
            None => bail!("declared remote {} is missing", remote.name),
        }
    }
    Ok(())
}

/// 让本地 remote 配置收敛到清单声明，包括清除遗留 push URL。
pub fn configure_declared_remotes(
    path: &Path,
    repository: &RepositoryRecord,
    _allow_stdin: bool,
) -> Result<()> {
    let git_repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    for remote in &repository.remotes {
        match git_repository.find_remote(&remote.name) {
            Ok(_) => {}
            Err(error) if error.code() == git2::ErrorCode::NotFound => {
                git_repository
                    .remote(&remote.name, &remote.fetch_url)
                    .with_context(|| format!("failed to add remote {}", remote.name))?;
            }
            Err(error) => return Err(error.into()),
        }
    }

    let mut config = git_repository
        .config()
        .context("failed to open repository config")?;
    for remote in &repository.remotes {
        let refspec = format!("+refs/heads/*:refs/remotes/{}/*", remote.name);
        config
            .set_str(&format!("remote.{}.fetch", remote.name), &refspec)
            .with_context(|| format!("failed to configure fetch refspec for {}", remote.name))?;

        let push_url = remote
            .push_url
            .as_deref()
            .filter(|push_url| *push_url != remote.fetch_url);
        let has_push_url = git_repository
            .find_remote(&remote.name)
            .with_context(|| format!("failed to read remote {}", remote.name))?
            .pushurl()
            .ok()
            .flatten()
            .is_some();
        if push_url.is_some() || has_push_url {
            git_repository
                .remote_set_pushurl(&remote.name, push_url)
                .with_context(|| format!("failed to configure push URL for {}", remote.name))?;
        }
    }
    Ok(())
}

/// 移除 HTTP(S) URL 中可能包含的用户名、密码或 token。
pub fn display_remote_url(raw: &str) -> String {
    portable_remote_url(raw)
}

/// 读取远端 fetch URL，并移除 HTTP(S) 凭据后返回。
fn remote_url(path: &Path, remote_name: &str) -> Result<Option<String>> {
    let repository = Repository::open(path)
        .with_context(|| format!("failed to open Git repository {}", path.display()))?;
    match repository.find_remote(remote_name) {
        Ok(remote) => Ok(remote.url().ok().map(portable_remote_url)),
        Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// 将本地路径尽量转换为可复制的绝对 file URL 或规范文本。
pub(super) fn portable_remote_url(raw: &str) -> String {
    let Ok(mut parsed) = Url::parse(raw) else {
        return raw.to_owned();
    };
    if matches!(parsed.scheme(), "http" | "https")
        && (!parsed.username().is_empty() || parsed.password().is_some())
    {
        let _ = parsed.set_username("");
        let _ = parsed.set_password(None);
    }
    parsed.to_string()
}

#[cfg(test)]
mod tests {
    use crate::model::{RemoteRecord, RepositoryRecord, now};

    use super::{configure_declared_remotes, display_remote_url};

    #[test]
    fn display_remote_url_removes_http_credentials() {
        assert_eq!(
            display_remote_url("https://user:secret@example.com/team/repository.git"),
            "https://example.com/team/repository.git"
        );
        assert_eq!(
            display_remote_url("git@example.com:team/repository.git"),
            "git@example.com:team/repository.git"
        );
    }

    #[test]
    fn declared_push_url_replaces_and_clears_local_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let repository = git2::Repository::init(directory.path()).unwrap();
        repository
            .remote("origin", "https://example.com/fetch.git")
            .unwrap();
        repository
            .remote_set_pushurl("origin", Some("https://example.com/stale.git"))
            .unwrap();

        let timestamp = now();
        let mut record = RepositoryRecord {
            name: "example".to_owned(),
            directory: "example".to_owned(),
            default_branch: "main".to_owned(),
            primary_remote: "origin".to_owned(),
            remotes: vec![RemoteRecord {
                name: "origin".to_owned(),
                fetch_url: "https://example.com/fetch.git".to_owned(),
                push_url: None,
            }],
            created_at: timestamp,
            synced_at: None,
        };

        configure_declared_remotes(directory.path(), &record, false).unwrap();
        assert!(
            repository
                .find_remote("origin")
                .unwrap()
                .pushurl()
                .unwrap()
                .is_none()
        );

        record.remotes[0].push_url = Some("https://example.com/push.git".to_owned());
        configure_declared_remotes(directory.path(), &record, false).unwrap();
        assert_eq!(
            repository.find_remote("origin").unwrap().pushurl().unwrap(),
            Some("https://example.com/push.git")
        );
    }
}
