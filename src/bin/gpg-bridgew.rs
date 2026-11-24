//! Same as gpg-bridge, but without a console window, for Task Scheduler and the like.
#![windows_subsystem = "windows"]

fn main() -> std::io::Result<()> {
    gpg_bridge::run()
}
