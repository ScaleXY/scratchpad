//! Runs the real `scratchpad` binary: version output, headless start-up, health check,
//! graceful shutdown and start-up errors.

use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_scratchpad");

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("scratchpad-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

struct Server {
    child: Child,
    dir: PathBuf,
    smtp: u16,
    web: u16,
}

impl Server {
    fn start(name: &str) -> Self {
        let dir = temp_dir(name);
        let (smtp, web) = (free_port(), free_port());
        let child = Command::new(BIN)
            .args(["--headless", "--data-dir"])
            .arg(&dir)
            .args([
                "--smtp-port",
                &smtp.to_string(),
                "--web-port",
                &web.to_string(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start scratchpad");
        Self {
            child,
            dir,
            smtp,
            web,
        }
    }

    fn healthcheck(&self) -> bool {
        Command::new(BIN)
            .args(["--healthcheck", "--data-dir"])
            .arg(&self.dir)
            .args(["--web-port", &self.web.to_string()])
            .status()
            .unwrap()
            .success()
    }

    fn wait_healthy(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.healthcheck() {
            assert!(Instant::now() < deadline, "server did not become healthy");
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn prints_version() {
    let output = Command::new(BIN).arg("--version").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        format!("scratchpad {}", env!("SCRATCHPAD_VERSION"))
    );
}

#[test]
fn headless_serves_smtp_and_web_and_stops_cleanly() {
    let mut server = Server::start("headless");
    server.wait_healthy();

    let smtp = TcpStream::connect(("127.0.0.1", server.smtp)).unwrap();
    smtp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut greeting = String::new();
    BufReader::new(smtp).read_line(&mut greeting).unwrap();
    assert!(greeting.starts_with("220 "), "{greeting}");

    let mut banner = String::new();
    let stdout = server.child.stdout.take().unwrap();
    BufReader::new(stdout).read_line(&mut banner).unwrap();
    assert!(banner.contains("headless"), "{banner}");

    #[cfg(unix)]
    {
        let status = Command::new("kill")
            .args(["-TERM", &server.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = server.child.try_wait().unwrap() {
                assert!(status.success(), "exit status {status}");
                break;
            }
            assert!(Instant::now() < deadline, "did not stop on SIGTERM");
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(!server.healthcheck());
    }
}

#[test]
fn headless_fails_when_the_web_port_is_taken() {
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let dir = temp_dir("port-taken");
    let output = Command::new(BIN)
        .args(["--headless", "--data-dir"])
        .arg(&dir)
        .args([
            "--smtp-port",
            &free_port().to_string(),
            "--web-port",
            &port.to_string(),
        ])
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("could not start the web interface"),
        "{stderr}"
    );
}

#[test]
fn healthcheck_fails_without_a_server() {
    let dir = temp_dir("no-server");
    let status = Command::new(BIN)
        .args(["--healthcheck", "--data-dir"])
        .arg(&dir)
        .args(["--web-port", &free_port().to_string()])
        .status()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!status.success());
}
