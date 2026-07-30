//! `batch-git` 可执行程序的最小入口。

fn main() {
    // 保留操作系统原始参数，避免 Git 参数中的非 UTF-8 字节被提前破坏。
    let code = batch_git::run(std::env::args_os().collect());
    // 将库层聚合的退出码原样返回给 shell，便于脚本判断结果。
    std::process::exit(code);
}
