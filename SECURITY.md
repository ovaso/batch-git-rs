# Security Policy

[简体中文](SECURITY.zh-CN.md)

## Supported versions

Security fixes are accepted for the latest release and the `main` branch only. Older releases are not backported unless maintainers announce otherwise.

## Reporting a vulnerability

Do not disclose exploitable details, credentials, private repository URLs, or unpatched vulnerabilities in a public issue. Contact the maintainers through GitHub Security Advisories private reporting. If private reporting is not yet enabled, contact a repository maintainer on GitHub and label the message “security report.” Include reproduction steps, affected versions, impact, and a feasible remediation when possible.

Maintainers aim to acknowledge reports within 7 days and, once the issue is reproducible, coordinate a fix, testing, and disclosure timeline. Resolved issues are documented in the release notes and both changelogs after a fixed release is available.

## Security boundaries

`batch-git` does not store credentials and removes user information from displayed HTTP(S) remote URLs. The machine protocol does not include raw Git child-process stdout/stderr by default and sanitizes HTTP(S) URL user-info in top-level diagnostics. The tool intentionally avoids implicit destructive Git operations. Vulnerabilities that bypass these guarantees, expose credentials, escape the workspace directory, overwrite unregistered files, or register unsafe native scheduler tasks should be reported under this policy.
