use clap::Parser;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::{env, io};

#[derive(Parser)]
#[command(name = "gpg-bridge")]
#[command(version, about)]
struct GpgBridge {
    /// Sets the listenning to bridge the extra socket
    #[arg(long, value_name("ADDRESS"))]
    extra: String,
    /// Sets the path to gnupg extra socket optionaly
    #[arg(long, value_name("PATH"))]
    extra_socket: Option<String>,
    /// Runs the program as a background daemon
    #[arg(long)]
    detach: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    pretty_env_logger::init();
    let cfg = GpgBridge::parse();
    if cfg.detach {
        let _ = gpg_bridge::ping_gpg_agent().await;

        let mut args = env::args();
        let mut cmd = Command::new(args.next().unwrap());
        for arg in args {
            if arg != "--detach" {
                cmd.arg(arg);
            }
        }
        return cmd
            .creation_flags(0x0000_0200 | 0x0000_0008 | 0x0400_0000)
            .spawn()
            .map(|_| ());
    }

    gpg_bridge::bridge(cfg.extra, cfg.extra_socket).await
}
