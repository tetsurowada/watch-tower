mod body;
mod cli;
mod docker;
mod hooks;
mod notify_compat;
mod process;
mod project;
mod server;
mod sink;
mod store;
mod text;

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use cli::{Command, ServeConfig, USAGE};
use docker::DockerCli;
use server::App;
use sink::Sink;
use sink::macos::Osascript;
use store::Store;

const OSASCRIPT_TIMEOUT: Duration = Duration::from_secs(10);
const DOCKER_TIMEOUT: Duration = Duration::from_secs(5);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args) {
        Ok(Command::Help) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Command::Serve(config)) => serve(config),
        Err(e) => {
            eprintln!("watch-tower: {e}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn serve(config: ServeConfig) -> ExitCode {
    let store = match open_store(Path::new(&config.db)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("watch-tower: {e}");
            return ExitCode::FAILURE;
        }
    };
    let server = match tiny_http::Server::http(&config.addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("watch-tower: failed to listen on {}: {e}", config.addr);
            return ExitCode::FAILURE;
        }
    };
    if let Some(addr) = server.server_addr().to_ip() {
        eprintln!("watch-tower: listening on {addr}");
    }

    let (sink, native) = choose_sink(&config.osascript);
    let app = Arc::new(App {
        sink,
        store,
        docker: Box::new(DockerCli::new(config.docker, DOCKER_TIMEOUT)),
        default_title: config.default_title,
        log_hooks: config.log_hooks,
    });

    if !native {
        server::run(server, app);
        return ExitCode::SUCCESS;
    }
    // Native notification callbacks need the main run loop, so the server moves aside.
    std::thread::spawn(move || server::run(server, app));
    run_main_loop()
}

fn open_store(db: &Path) -> Result<Store, String> {
    if let Some(dir) = db.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    }
    Store::open(db).map_err(|e| format!("failed to open {}: {e}", db.display()))
}

/// Native notifications inside the .app bundle, osascript everywhere else.
/// The flag says whether the main thread has to run the native run loop.
fn choose_sink(osascript: &str) -> (Box<dyn Sink>, bool) {
    #[cfg(target_os = "macos")]
    if let Some(native) = sink::native::UserNotifications::new() {
        return (Box::new(native), true);
    }
    eprintln!("watch-tower: notifications through {osascript}");
    (
        Box::new(Osascript::new(osascript, OSASCRIPT_TIMEOUT)),
        false,
    )
}

#[cfg(target_os = "macos")]
fn run_main_loop() -> ! {
    sink::native::run_main_loop()
}

#[cfg(not(target_os = "macos"))]
fn run_main_loop() -> ! {
    unreachable!("native notifications only exist on macOS")
}
