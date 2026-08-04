# 兼容性

## 工具链

| 项目 | 支持基线 | 说明 |
|---|---:|---|
| Rust | 1.85 | Edition 2024 的最低工具链；CI 同时测试此版本和 stable。 |
| Git | 2.30+ | 需要系统 Git 处理 add、commit、restore/read-tree unstage、clone、fetch、pull、push、merge 与透传；建议使用维护中的最新版。 |
| workspace.toml | version 1 | 当前唯一支持的清单 schema。 |

## 平台

| 平台 | CLI | schedule 后端 | CI 构建 |
|---|---|---|---|
| Linux x86_64 | 支持 | `systemd --user` | 支持 |
| macOS arm64/x86_64 | 支持 | `launchd` | 支持 |
| Windows x86_64 | 支持 | Task Scheduler | 支持 |

CI 在三种操作系统构建 release 二进制；真实注册行为仍依赖 runner 用户权限和平台服务可用性。
发布前应在目标平台执行 `schedule doctor`、`generate`，并按需人工验证 register/unregister。

## 已知平台约束

- launchd 的 cron 秒字段必须为 `0`；
- Windows 不支持 cron，`--every` 仅允许 `1m` 至 `31d`；
- Linux schedule 需要可用的用户级 systemd session；
- 默认时区由操作系统决定。设置 `BATCH_GIT_TZ` 后必须重新 register；
- 交互 Git 子进程需要 `--jobs 1` 和可用终端；`--output json|jsonl` 与
  `--non-interactive` 会有意禁用终端提示。
- `commit` 使用 `-m`，不会启动普通编辑器，但会遵循各仓库的 hook 与签名配置。依赖交互式
  hook 或签名程序时仍须使用文本模式和 `--jobs 1`；自定义程序直接访问 TTY 的行为不受
  `GIT_TERMINAL_PROMPT=0` 完整约束。
- `--timeout` 终止直接启动的 Git 子进程；batch-git 会报告 timeout，但不能保证终止 Git
  再派生的全部认证、传输、filter、hook 或签名进程，这些后代可能继续运行；它也不限制等待
  工作区锁的时间。commit timeout 后本地 ref 可能已经更新，必须检查仓库实际状态。
- 已注册的原生 schedule 始终以非交互 Git child 运行；需要使用预先配置的凭据，不能依赖
  终端认证提示。

兼容性变更会先写入 `CHANGELOG.md` 的 `Unreleased`。不在本表中的系统或 Git 版本可尝试使用，
但不构成发布支持承诺。
