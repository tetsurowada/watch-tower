mod body;
mod cli;
mod notify_compat;
mod server;
mod sink;

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use cli::{Command, ServeConfig, USAGE};
use server::App;
use sink::macos::Osascript;

const OSASCRIPT_TIMEOUT: Duration = Duration::from_secs(10);

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
        default_title: config.default_title,
    });
    server::run(server, app);
    ExitCode::SUCCESS
}
