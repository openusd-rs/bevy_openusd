use std::{path::PathBuf, time::Duration};
use serde::{Deserialize, Serialize};
use crate::capture_tools::CaptureTools;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Ping,
    Screenshot { output: PathBuf, #[serde(default = "default_ui")] include_ui: bool },
}
fn default_ui() -> bool { true }

const HELP: &str = "usdview [SCENE.usd]\nusdview instances\nusdview screenshot --output FILE.png [--instance PID] [--scene-only]\n\nScreenshots target an already-running viewer. UI is included by default.\nOutputs are never overwritten. Commands return JSON; failures exit nonzero.";

pub fn run_cli() -> Result<bool, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else { return Ok(false) };
    if command == "--help" || command == "-h" { println!("{HELP}"); return Ok(true); }
    if command != "instances" && command != "screenshot" { return Ok(false); }
    #[cfg(unix)]
    { local::run(&args)?; Ok(true) }
    #[cfg(not(unix))]
    { Err("Running-instance control is currently supported on Unix only".into()) }
}

#[cfg(unix)]
pub use local::Server;

#[cfg(not(unix))]
pub struct Server;
#[cfg(not(unix))]
impl Server {
    pub fn start(_: CaptureTools, _: eframe::egui::Context) -> Result<Self, String> {
        Err("Running-instance control is currently supported on Unix only".into())
    }
}

#[cfg(unix)]
mod local {
    use super::*;
    use std::{io::{BufRead, BufReader, Read, Write}, os::unix::{net::{UnixListener, UnixStream}, fs::{DirBuilderExt, MetadataExt, PermissionsExt}},
        sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}}};

    fn directory() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let uid = rustix::process::getuid().as_raw();
        let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let path = base.join(format!("usdview-{uid}"));
        match std::fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => {}, Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}, Err(error) => return Err(error.into()),
        }
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
            return Err("Viewer runtime directory must be owned by this user with mode 0700".into());
        }
        Ok(path)
    }

    fn line(stream: &mut UnixStream) -> Result<String, Box<dyn std::error::Error>> {
        let mut text = String::new();
        BufReader::new(stream).take(65537).read_line(&mut text)?;
        if text.len() > 65536 || !text.ends_with('\n') { return Err("Expected one JSON line, at most 64 KiB".into()); }
        Ok(text)
    }

    fn call(path: &std::path::Path, request: &Request, timeout: u64) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let mut stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(Duration::from_secs(timeout)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        writeln!(stream, "{}", serde_json::to_string(request)?)?;
        Ok(serde_json::from_str(&line(&mut stream)?)?)
    }

    fn instances() -> Result<Vec<(u32, PathBuf)>, Box<dyn std::error::Error>> {
        let mut found = vec![];
        for entry in std::fs::read_dir(directory()?)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("sock") { continue; }
            let Some(pid) = path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<u32>().ok()) else { continue };
            if call(&path, &Request::Ping, 1).is_ok_and(|value| value["pid"].as_u64() == Some(u64::from(pid))) { found.push((pid, path)); }
        }
        found.sort_by_key(|(pid, _)| *pid);
        Ok(found)
    }

    pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
        if args[0] == "instances" {
            if args.len() != 1 { return Err("Usage: usdview instances".into()); }
            let rows: Vec<_> = instances()?.into_iter().map(|(pid, socket)| serde_json::json!({"pid":pid,"socket":socket})).collect();
            println!("{}", serde_json::to_string(&rows)?); return Ok(());
        }
        let mut output = None; let mut pid = None; let mut include_ui = true;
        let mut words = args[1..].iter();
        while let Some(word) = words.next() {
            match word.as_str() {
                "--output" if output.is_none() => output = Some(PathBuf::from(words.next().ok_or("Missing output path")?)),
                "--instance" if pid.is_none() => pid = Some(words.next().ok_or("Missing instance PID")?.parse::<u32>()?),
                "--scene-only" => include_ui = false,
                _ => return Err(format!("Unknown or repeated argument: {word}\n{HELP}").into()),
            }
        }
        let output = output.ok_or("--output FILE.png is required")?;
        let output = if output.is_absolute() { output } else { std::env::current_dir()?.join(output) };
        let socket = if let Some(pid) = pid { directory()?.join(format!("{pid}.sock")) } else {
            let running = instances()?;
            if running.len() != 1 { return Err(format!("Found {} viewers; use usdview instances and specify --instance PID", running.len()).into()); }
            running[0].1.clone()
        };
        let response = call(&socket, &Request::Screenshot { output, include_ui }, 45)?;
        println!("{response}");
        if response["ok"] != true { return Err("Screenshot failed".into()); }
        Ok(())
    }

    struct Worker(Arc<AtomicUsize>);
    impl Drop for Worker { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::Relaxed); } }

    pub struct Server { path: PathBuf, stop: Arc<AtomicBool> }
    impl Server {
        pub fn start(tools: CaptureTools, ctx: eframe::egui::Context) -> Result<Self, String> {
            let path = directory().map_err(|e| e.to_string())?.join(format!("{}.sock", std::process::id()));
            let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
            listener.set_nonblocking(true).map_err(|e| e.to_string())?;
            let stop = Arc::new(AtomicBool::new(false)); let stopped = stop.clone();
            let workers = Arc::new(AtomicUsize::new(0));
            std::thread::spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                            if workers.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| (count < 16).then_some(count + 1)).is_err() {
                                let _ = writeln!(stream, "{{\"ok\":false,\"error\":\"API connection limit reached\"}}");
                                continue;
                            }
                            let worker = Worker(workers.clone());
                            let tools = tools.clone(); let ctx = ctx.clone();
                            std::thread::spawn(move || {
                            let _worker = worker;
                            let result = (|| -> Result<serde_json::Value, String> {
                                let request: Request = serde_json::from_str(&line(&mut stream).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                                match request {
                                    Request::Ping => Ok(serde_json::json!({"ok":true,"pid":std::process::id(),"version":1})),
                                    Request::Screenshot { output, include_ui } => {
                                        let reply = tools.screenshot(output, include_ui)?;
                                        ctx.request_repaint();
                                        let path = reply.recv_timeout(Duration::from_secs(40)).map_err(|e| e.to_string())??;
                                        Ok(serde_json::json!({"ok":true,"output":path,"include_ui":include_ui}))
                                    }
                                }
                            })();
                            let response = result.unwrap_or_else(|error| serde_json::json!({"ok":false,"error":error}));
                            let _ = writeln!(stream, "{response}");
                            });
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(20)),
                        Err(_) => break,
                    }
                }
            });
            Ok(Self { path, stop })
        }
    }
    impl Drop for Server {
        fn drop(&mut self) { self.stop.store(true, Ordering::Relaxed); let _ = std::fs::remove_file(&self.path); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn api_defaults_to_ui_and_rejects_unknown_fields() {
        let Request::Screenshot { include_ui, .. } = serde_json::from_str(r#"{"command":"screenshot","output":"/tmp/test.png"}"#).unwrap() else { panic!() };
        assert!(include_ui);
        assert!(serde_json::from_str::<Request>(r#"{"command":"screenshot","output":"/tmp/test.png","typo":true}"#).is_err());
    }
}
