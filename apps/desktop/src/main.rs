#![windows_subsystem = "windows"]

mod deps;
mod coverflow;
mod view_models;
mod bridge;
mod bootstrap;
mod worker;
mod native;
mod artwork;

/// 启动原生软件渲染客户端。
fn main() {
    if let Err(error) = bootstrap::runapp() { eprintln!("{error}"); }
}
