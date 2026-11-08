mod body;
mod cli;
mod docker;
mod hooks;
mod notify_compat;
mod process;
mod server;
mod sink;
mod store;

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use cli::{Command, ServeConfig, USAGE};
use docker::DockerCli;
use server::App;
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
    let db = Path::new(&config.db);
    if let Some(dir) = db.parent().filter(|d| !d.as_os_str().is_empty())
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        eprintln!("watch-tower: failed to create {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    let store = match Store::open(db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("watch-tower: failed to open {}: {e}", db.display());
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

    let app = Arc::new(App {
        sink: Box::new(Osascript::new(config.osascript, OSASCRIPT_TIMEOUT)),
        store,
        docker: Box::new(DockerCli::new(config.docker, DOCKER_TIMEOUT)),
        default_title: config.default_title,
        log_hooks: config.log_hooks,
    });
    server::run(server, app);
    ExitCode::SUCCESS
}
