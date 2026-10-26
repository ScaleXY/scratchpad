//! Scratchpad: email testing and debugging.
//!
//! * `scratchpad` starts the desktop app (with the web interface on localhost).
//! * `scratchpad --headless` runs the SMTP server and web interface without a GUI,
//!   e.g. on a server or in a container.

#![cfg_attr(
    all(windows, feature = "gui", not(debug_assertions)),
    windows_subsystem = "windows"
)]

#[cfg(feature = "gui")]
mod gui;
mod logging;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use scratchpad_core::{Scratchpad, Settings};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "scratchpad",
    version = env!("SCRATCHPAD_VERSION"),
    about = "Scratchpad - email testing and debugging for developers"
)]
pub struct Cli {
    /// Run without the desktop GUI and serve the web interface (for servers and containers).
    #[arg(long, env = "SCRATCHPAD_HEADLESS")]
    headless: bool,

    /// Directory holding settings and the email database.
    #[arg(long, env = "SCRATCHPAD_DATA_DIR")]
    data_dir: Option<PathBuf>,

    /// SMTP listen address (overrides settings). Use 0.0.0.0 to accept remote connections.
    #[arg(long, env = "SCRATCHPAD_SMTP_HOST")]
    smtp_host: Option<String>,

    /// SMTP port (overrides settings, default 2525).
    #[arg(long, env = "SCRATCHPAD_SMTP_PORT")]
    smtp_port: Option<u16>,

    /// Web interface listen address (overrides settings).
    #[arg(long, env = "SCRATCHPAD_WEB_HOST")]
    web_host: Option<String>,

    /// Web interface port (overrides settings, default 8025).
    #[arg(long, env = "SCRATCHPAD_WEB_PORT")]
    web_port: Option<u16>,

    /// Require HTTP basic auth for the web interface, as `user:password`.
    #[arg(long, env = "SCRATCHPAD_WEB_AUTH")]
    web_auth: Option<String>,

    /// Public base URL used in share links (e.g. `https://scratchpad.example.com`).
    #[arg(long, env = "SCRATCHPAD_PUBLIC_URL")]
    public_url: Option<String>,

    /// Unlock the synced credential vault at startup (for headless servers).
    #[arg(long, env = "SCRATCHPAD_MASTER_PASSWORD", hide_env_values = true)]
    master_password: Option<String>,

    /// Disable the web interface in desktop mode.
    #[arg(long)]
    no_web: bool,

    /// Log filter, e.g. `info` or `scratchpad_core=debug`.
    #[arg(
        long,
        env = "SCRATCHPAD_LOG",
        default_value = "info,iced_winit=warn,iced_wgpu=warn,wgpu_core=warn,wgpu_hal=warn,naga=warn,cosmic_text=warn"
    )]
    log: String,

    /// Log output format on stderr.
    #[arg(
        long,
        env = "SCRATCHPAD_LOG_FORMAT",
        value_enum,
        default_value = "text"
    )]
    log_format: logging::LogFormat,

    /// Check that a running instance's web interface answers, then exit 0 (healthy) or 1.
    /// Uses the same settings and flags as the server (for container health checks).
    #[arg(long)]
    healthcheck: bool,
}

impl Cli {
    fn apply(&self, settings: &mut Settings) {
        if let Some(host) = &self.smtp_host {
            settings.smtp_host = host.clone();
        }
        if let Some(port) = self.smtp_port {
            settings.smtp_port = port;
        }
        if let Some(host) = &self.web_host {
            settings.web_host = host.clone();
        }
        if let Some(port) = self.web_port {
            settings.web_port = port;
        }
        if let Some(auth) = &self.web_auth {
            let (user, pass) = auth.split_once(':').unwrap_or((auth, ""));
            settings.web_username = user.to_string();
            settings.web_password = pass.to_string();
        }
        if let Some(url) = &self.public_url {
            settings.public_url = url.clone();
        }
    }
}

/// Release GUI builds use the Windows GUI subsystem (no console window). Re-attach to the
/// console of the launching terminal so `--headless`, `--help` and `--version` print output.
#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE,
    };
    // SAFETY: plain Win32 calls without pointers; failure just means there is no parent console.
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    attach_parent_console();
    let cli = Cli::parse();
    let data_dir = cli
        .data_dir
        .clone()
        .unwrap_or_else(scratchpad_core::default_data_dir);

    if cli.healthcheck {
        let mut settings = Settings::load(&data_dir).unwrap_or_default();
        cli.apply(&mut settings);
        std::process::exit(match healthcheck(&settings.web_host, settings.web_port) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("unhealthy: {error}");
                1
            }
        });
    }

    // The desktop app also logs to a file: it has no console on Windows.
    let desktop = cfg!(feature = "gui") && !cli.headless;
    let log_dir = data_dir.join("logs");
    if let Some(path) = logging::init(&cli.log, cli.log_format, desktop.then_some(&*log_dir)) {
        tracing::info!("logging to {}", path.display());
    }
    tracing::info!("Scratchpad {}", env!("SCRATCHPAD_VERSION"));

    let scratchpad = Scratchpad::open(&data_dir, |settings| cli.apply(settings))?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("scratchpad")
        .build()?;

    #[cfg(feature = "gui")]
    if !cli.headless {
        let web = !cli.no_web;
        runtime.block_on(start_services(
            &scratchpad,
            web,
            false,
            cli.master_password.clone(),
        ))?;
        let handle = runtime.handle().clone();
        // Keep the runtime alive on background threads while iced owns the main thread.
        let result = gui::run(scratchpad, handle);
        runtime.shutdown_timeout(std::time::Duration::from_secs(1));
        return result;
    }

    #[cfg(not(feature = "gui"))]
    if !cli.headless {
        tracing::info!("built without the desktop GUI, running headless");
    }

    runtime.block_on(async move {
        start_services(&scratchpad, true, true, cli.master_password.clone()).await?;
        print_banner(&scratchpad);
        shutdown_signal().await;
        tracing::info!("shutting down");
        anyhow::Ok(())
    })
}

/// Starts the SMTP server and (optionally) the web interface.
async fn start_services(
    scratchpad: &Arc<Scratchpad>,
    web: bool,
    headless: bool,
    master_password: Option<String>,
) -> anyhow::Result<()> {
    scratchpad.start();
    if let Some(password) = master_password.filter(|p| !p.is_empty()) {
        match scratchpad.vault_unlock(password).await {
            Ok(()) => tracing::info!("credential vault unlocked"),
            Err(error) => tracing::warn!(%error, "could not unlock the credential vault"),
        }
    }
    if web {
        let settings = scratchpad.settings();
        match scratchpad_web::WebServer::bind(
            scratchpad.clone(),
            &settings.web_host,
            settings.web_port,
        )
        .await
        {
            Ok(server) => {
                scratchpad.set_web_url(Some(server.url()));
                tokio::spawn(async move {
                    if let Err(error) = server.serve().await {
                        tracing::error!(%error, "web server stopped");
                    }
                });
            }
            Err(error) if headless => {
                anyhow::bail!(
                    "could not start the web interface on {}:{}: {error:#}",
                    settings.web_host,
                    settings.web_port
                );
            }
            Err(error) => {
                // Desktop: fall back to an ephemeral port so previews and sharing keep working.
                tracing::warn!(%error, "falling back to a random web port");
                let server =
                    scratchpad_web::WebServer::bind(scratchpad.clone(), &settings.web_host, 0)
                        .await?;
                scratchpad.set_web_url(Some(server.url()));
                tokio::spawn(server.serve());
            }
        }
    }
    // Give the SMTP supervisor a moment to bind so the banner shows the real state.
    for _ in 0..50 {
        let status = scratchpad.status();
        if status.smtp_running || status.smtp_error.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    Ok(())
}

fn print_banner(scratchpad: &Scratchpad) {
    let status = scratchpad.status();
    let settings = scratchpad.settings();
    println!("Scratchpad {} (headless)", status.version);
    match &status.smtp_error {
        Some(error) => println!("  SMTP  : {error}"),
        None => println!("  SMTP  : {}", status.smtp_address),
    }
    if let Some(url) = &status.web_url {
        let auth = if settings.web_auth_enabled() {
            " (basic auth enabled)"
        } else {
            ""
        };
        println!("  Web   : {url}{auth}");
        if settings.web_host != "127.0.0.1"
            && settings.web_host != "localhost"
            && !settings.web_auth_enabled()
        {
            println!("  WARNING: the web interface is reachable from the network without authentication. Use --web-auth user:pass.");
        }
    }
    println!("  Data  : {}", status.data_dir);
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

/// GETs `/healthz` from the web interface with a plain TCP request (no TLS: it listens
/// on plain HTTP), so the container image needs no curl.
fn healthcheck(host: &str, port: u16) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;

    let host = match host {
        "0.0.0.0" | "" => "127.0.0.1",
        "::" | "[::]" => "::1",
        other => other,
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let address = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("cannot resolve {host}"))?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let host_header = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    write!(
        stream,
        "GET /healthz HTTP/1.1\r\nHost: {host_header}\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = String::new();
    stream.take(4096).read_to_string(&mut response)?;
    let status = response.lines().next().unwrap_or_default();
    if status.split_whitespace().nth(1) == Some("200") {
        Ok(())
    } else {
        anyhow::bail!("{address} answered {status:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn flags_override_settings() {
        let cli = Cli::parse_from([
            "scratchpad",
            "--headless",
            "--smtp-host",
            "0.0.0.0",
            "--smtp-port",
            "2626",
            "--web-port",
            "9025",
            "--web-auth",
            "admin:pa:ss",
            "--public-url",
            "https://scratchpad.example.com",
        ]);
        let mut settings = Settings::default();
        cli.apply(&mut settings);
        assert_eq!(settings.smtp_address(), "0.0.0.0:2626");
        assert_eq!(settings.web_port, 9025);
        assert_eq!(settings.web_username, "admin");
        // Only the first colon separates user and password.
        assert_eq!(settings.web_password, "pa:ss");
        assert_eq!(settings.public_url, "https://scratchpad.example.com");
        assert!(settings.web_auth_enabled());
    }

    #[test]
    fn healthcheck_reports_down_servers() {
        // Find a port nothing listens on.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(healthcheck("127.0.0.1", port).is_err());
    }
}
