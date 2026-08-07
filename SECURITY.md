# Security Policy

[简体中文](SECURITY.zh-CN.md)

## Maintenance scope

This project makes no fixed support-window, response-time, disclosure-schedule, or backport
commitment. Reports concerning the current `main` branch or a tagged GitHub Release may be evaluated
on a best-effort basis according to reproducibility, impact, and maintainer availability.

## Reporting a vulnerability

Do not disclose exploitable details, credentials, private repository URLs, or unpatched vulnerabilities in a public issue. Contact the maintainers through GitHub Security Advisories private reporting. If private reporting is not yet enabled, contact a repository maintainer on GitHub and label the message “security report.” Include reproduction steps, affected versions, impact, and a feasible remediation when possible.

## Security boundaries

`batch-git` does not store credentials and removes user information from displayed HTTP(S) remote URLs. The machine protocol does not include raw Git child-process stdout/stderr by default and sanitizes HTTP(S) URL user-info in top-level diagnostics. The tool intentionally avoids implicit destructive Git operations. Vulnerabilities that cross these documented boundaries, expose credentials, escape the workspace directory, overwrite unregistered files, or register unsafe native scheduler tasks should be reported under this policy.
