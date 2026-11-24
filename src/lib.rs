use clap::Parser;
use log::{debug, error, warn};
use std::convert::TryInto;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use std::{io, str};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

#[derive(Parser)]
#[command(name = "gpg-bridge")]
#[command(version, about)]
struct Args {
    /// Sets the listenning to bridge the extra socket
    #[arg(long, value_name("ADDRESS"))]
    extra: String,
    /// Sets the path to gnupg extra socket optionaly
    #[arg(long, value_name("PATH"))]
    extra_socket: Option<String>,
    /// Appends logs to this file instead of stderr
    #[arg(long, value_name("PATH"))]
    log_file: Option<PathBuf>,
}

pub fn run() -> io::Result<()> {
    let args = Args::parse();
    let mut logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"));
    if let Some(path) = &args.log_file {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        logger
            .target(env_logger::Target::Pipe(Box::new(file)))
            .write_style(env_logger::WriteStyle::Never);
    }
    logger.init();

    let res = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(bridge(args.extra, args.extra_socket));
    if let (Err(e), Some(_)) = (&res, &args.log_file) {
        // gpg-bridgew has no stderr, make sure fatal errors reach the log file.
        error!("{}", e);
    }
    res
}

/// Runs gpg tools without a console window, which would pop up when we have none.
fn gpg_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

type Nonce = (u16, [u8; 16]);

struct AgentMeta {
    path: Option<String>,
    args: Option<Nonce>,
}

async fn load_gpg_socket_path() -> io::Result<String> {
    let output = gpg_command("gpgconf")
        .arg("--list-dirs")
        .arg("agent-extra-socket")
        .output()
        .await?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "failed to load extra socket: {:?}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

async fn ping_gpg_agent() -> io::Result<()> {
    let output = gpg_command("gpg-connect-agent")
        .arg("/bye")
        .output()
        .await?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "failed to start gpg-agent: {:?}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn data_err(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Loads a libassuan socket file: "<port>\n" followed by a 16 byte nonce.
async fn load_port_nonce(path: &str) -> io::Result<Nonce> {
    if !Path::new(path).exists() {
        ping_gpg_agent().await?;
    }
    let buffer = tokio::fs::read(path).await?;
    if buffer.starts_with(b"!<socket >") {
        return Err(data_err(
            "cygwin/msys socket files are not supported, use Gpg4win's gpg-agent",
        ));
    }
    if buffer.len() <= 16 {
        return Err(data_err("socket file too short"));
    }
    let (left, right) = buffer.split_at(buffer.len() - 16);
    let port = str::from_utf8(left)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .ok_or_else(|| data_err("invalid port in socket file"))?;
    Ok((port, right.try_into().unwrap()))
}

/// Returns the cached port and nonce, loading them if needed. Callers hold the lock, so
/// concurrent connections never load the socket file twice.
async fn current_nonce(m: &mut AgentMeta) -> io::Result<Nonce> {
    if let Some(args) = m.args {
        return Ok(args);
    }
    if m.path.is_none() {
        m.path = Some(load_gpg_socket_path().await?);
    }
    let args = load_port_nonce(m.path.as_ref().unwrap()).await?;
    m.args = Some(args);
    Ok(args)
}

/// Connects to the agent and waits for its greeting. The agent closes the connection without
/// greeting when the nonce is stale, so a successful return means the nonce was accepted.
async fn connect_agent((port, nonce): Nonce) -> io::Result<(TcpStream, Vec<u8>)> {
    let handshake = async {
        let mut agent = TcpStream::connect(("127.0.0.1", port)).await?;
        agent.write_all(&nonce).await?;
        let mut greeting = vec![0; 1024];
        let n = agent.read(&mut greeting).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "agent closed connection, stale nonce?",
            ));
        }
        greeting.truncate(n);
        Ok((agent, greeting))
    };
    timeout(Duration::from_secs(10), handshake)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "agent did not greet"))?
}

async fn delegate(mut client: TcpStream, meta: Arc<Mutex<AgentMeta>>) -> io::Result<()> {
    let args = current_nonce(&mut *meta.lock().await).await?;
    let (mut agent, greeting) = match connect_agent(args).await {
        Ok(conn) => conn,
        Err(e) => {
            warn!("failed to connect to agent, retrying: {}", e);
            // gpg-agent was likely restarted. Restart it and reload while holding the lock, so
            // concurrent connections wait for the new nonce instead of caching the stale socket
            // file, and only the first one pings.
            let mut m = meta.lock().await;
            if m.args == Some(args) {
                m.args = None;
                let _ = ping_gpg_agent().await;
            }
            let args = current_nonce(&mut m).await?;
            drop(m);
            connect_agent(args).await?
        }
    };
    client.write_all(&greeting).await?;
    match tokio::io::copy_bidirectional(&mut client, &mut agent).await {
        Ok((received, replied)) => debug!(
            "connection finished, received {}, replied {}",
            received,
            replied + greeting.len() as u64
        ),
        // Peers often reset instead of closing gracefully once the session is over.
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
            ) =>
        {
            debug!("connection reset: {}", e)
        }
        Err(e) => return Err(e),
    }
    Ok(())
}

/// A bridge that forwards all requests from a TCP address to gpg-agent extra socket on Windows.
///
/// `to_path` should point to the path of gnupg UDS.
async fn bridge(from_addr: String, to_path: Option<String>) -> io::Result<()> {
    // Every client gets the agent's nonce injected, so never expose it beyond this machine.
    let addrs: Vec<_> = tokio::net::lookup_host(&from_addr).await?.collect();
    if let Some(addr) = addrs.iter().find(|a| !a.ip().is_loopback()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to listen on non-loopback address {}", addr),
        ));
    }
    let listener = TcpListener::bind(&addrs[..]).await?;
    // Attempt to setup gpg-agent if it's not up yet.
    let _ = ping_gpg_agent().await;
    let meta = Arc::new(Mutex::new(AgentMeta {
        path: to_path,
        args: None,
    }));
    loop {
        let conn = match listener.accept().await {
            Ok((conn, _)) => conn,
            Err(e) => {
                error!("failed to accept connection: {}", e);
                sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let meta = meta.clone();
        tokio::spawn(async move {
            if let Err(e) = delegate(conn, meta).await {
                error!("failed to delegate stream: {}", e);
            }
        });
    }
}
