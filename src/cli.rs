use std::fmt;

pub const USAGE: &str = "\
Usage:
  watch-tower serve [flags]

Flags for serve:
  -addr string           listen address (default \":4567\")
  -osascript string      osascript executable (default \"osascript\")
  -default-title string  title used when none is given (default \"Claude Code\")
  -db string             SQLite database
                         (default \"~/Library/Application Support/watch-tower/watch-tower.db\")
  -docker string         docker executable (default \"docker\")
  -log-hooks             log every received hook payload to stderr
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeConfig {
    pub addr: String,
    pub osascript: String,
    pub default_title: String,
    pub db: String,
    pub docker: String,
    pub log_hooks: bool,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            addr: normalize_addr(":4567"),
            osascript: "osascript".to_string(),
            default_title: "Claude Code".to_string(),
            db: default_db(),
            docker: "docker".to_string(),
            log_hooks: false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Serve(ServeConfig),
    Help,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CliError {
    MissingCommand,
    UnknownCommand(String),
    UnknownFlag(String),
    MissingValue(String),
    UnexpectedArgument(String),
    InvalidValue(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCommand => write!(f, "no command given"),
            Self::UnknownCommand(c) => write!(f, "unknown command: {c}"),
            Self::UnknownFlag(flag) => write!(f, "flag provided but not defined: {flag}"),
            Self::MissingValue(flag) => write!(f, "flag needs an argument: {flag}"),
            Self::UnexpectedArgument(a) => write!(f, "unexpected argument: {a}"),
            Self::InvalidValue(a) => write!(f, "invalid value for flag: {a}"),
        }
    }
}

/// Parses the arguments after the program name.
pub fn parse(args: &[String]) -> Result<Command, CliError> {
    let Some((command, rest)) = args.split_first() else {
        return Err(CliError::MissingCommand);
    };
    match command.as_str() {
        "serve" => parse_serve(rest),
        "help" | "-h" | "-help" | "--help" => Ok(Command::Help),
        other => Err(CliError::UnknownCommand(other.to_string())),
    }
}

fn parse_serve(args: &[String]) -> Result<Command, CliError> {
    let mut config = ServeConfig::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            if let Some(extra) = iter.next() {
                return Err(CliError::UnexpectedArgument(extra.clone()));
            }
            break;
        }
        let Some(flag) = arg
            .strip_prefix("--")
            .or_else(|| arg.strip_prefix('-'))
            .filter(|f| !f.is_empty())
        else {
            return Err(CliError::UnexpectedArgument(arg.clone()));
        };
        let (name, inline) = match flag.split_once('=') {
            Some((name, value)) => (name, Some(value.to_string())),
            None => (flag, None),
        };
        if matches!(name, "h" | "help") {
            return Ok(Command::Help);
        }
        if name == "log-hooks" {
            config.log_hooks = match inline.as_deref() {
                None | Some("true" | "1") => true,
                Some("false" | "0") => false,
                Some(_) => return Err(CliError::InvalidValue(arg.clone())),
            };
            continue;
        }
        let target = match name {
            "addr" => &mut config.addr,
            "osascript" => &mut config.osascript,
            "default-title" => &mut config.default_title,
            "db" => &mut config.db,
            "docker" => &mut config.docker,
            _ => return Err(CliError::UnknownFlag(arg.clone())),
        };
        *target = match inline {
            Some(value) => value,
            None => iter
                .next()
                .cloned()
                .ok_or_else(|| CliError::MissingValue(arg.clone()))?,
        };
    }
    config.addr = normalize_addr(&config.addr);
    Ok(Command::Serve(config))
}

fn default_db() -> String {
    match std::env::var_os("HOME") {
        Some(home) => std::path::Path::new(&home)
            .join("Library/Application Support/watch-tower/watch-tower.db")
            .to_string_lossy()
            .into_owned(),
        None => "watch-tower.db".to_string(),
    }
}

/// `:4567` listens on every interface, as Go's `net.Listen` does.
pub fn normalize_addr(addr: &str) -> String {
    if addr.starts_with(':') {
        format!("0.0.0.0{addr}")
    } else {
        addr.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn serve(list: &[&str]) -> ServeConfig {
        match parse(&args(list)) {
            Ok(Command::Serve(c)) => c,
            other => panic!("expected serve config, got {other:?}"),
        }
    }

    #[test]
    fn defaults() {
        let c = serve(&["serve"]);
        assert_eq!(c.addr, "0.0.0.0:4567");
        assert_eq!(c.osascript, "osascript");
        assert_eq!(c.default_title, "Claude Code");
    }

    #[test]
    fn flag_forms() {
        assert_eq!(serve(&["serve", "-addr", ":1"]).addr, "0.0.0.0:1");
        assert_eq!(serve(&["serve", "-addr=:2"]).addr, "0.0.0.0:2");
        assert_eq!(
            serve(&["serve", "--addr", "127.0.0.1:3"]).addr,
            "127.0.0.1:3"
        );
        assert_eq!(serve(&["serve", "--addr=[::1]:4"]).addr, "[::1]:4");
    }

    #[test]
    fn all_flags() {
        let c = serve(&[
            "serve",
            "-osascript",
            "/bin/fake",
            "-default-title=Hello world",
        ]);
        assert_eq!(c.osascript, "/bin/fake");
        assert_eq!(c.default_title, "Hello world");
    }

    #[test]
    fn storage_and_docker_flags() {
        let c = serve(&["serve", "-db", "/tmp/x.db", "-docker=/bin/fake-docker"]);
        assert_eq!(c.db, "/tmp/x.db");
        assert_eq!(c.docker, "/bin/fake-docker");
        assert!(serve(&["serve"]).db.ends_with("watch-tower/watch-tower.db"));
    }

    #[test]
    fn bool_flag() {
        assert!(!serve(&["serve"]).log_hooks);
        assert!(serve(&["serve", "-log-hooks"]).log_hooks);
        assert!(serve(&["serve", "--log-hooks=true"]).log_hooks);
        assert!(!serve(&["serve", "-log-hooks", "-log-hooks=false"]).log_hooks);
        // A bool flag does not consume the next argument.
        assert_eq!(
            serve(&["serve", "-log-hooks", "-addr", ":9"]).addr,
            "0.0.0.0:9"
        );
        assert_eq!(
            parse(&args(&["serve", "-log-hooks=yes"])),
            Err(CliError::InvalidValue("-log-hooks=yes".into()))
        );
    }

    #[test]
    fn inline_value_may_be_empty_or_contain_equals() {
        assert_eq!(serve(&["serve", "-default-title="]).default_title, "");
        assert_eq!(serve(&["serve", "-default-title=a=b"]).default_title, "a=b");
    }

    #[test]
    fn help() {
        for list in [
            &["serve", "-h"][..],
            &["serve", "--help"],
            &["help"],
            &["-h"],
        ] {
            assert_eq!(parse(&args(list)), Ok(Command::Help), "{list:?}");
        }
    }

    #[test]
    fn errors() {
        assert_eq!(parse(&[]), Err(CliError::MissingCommand));
        assert_eq!(
            parse(&args(&["start"])),
            Err(CliError::UnknownCommand("start".into()))
        );
        assert_eq!(
            parse(&args(&["serve", "-port", "1"])),
            Err(CliError::UnknownFlag("-port".into()))
        );
        assert_eq!(
            parse(&args(&["serve", "-addr"])),
            Err(CliError::MissingValue("-addr".into()))
        );
        assert_eq!(
            parse(&args(&["serve", "extra"])),
            Err(CliError::UnexpectedArgument("extra".into()))
        );
        assert_eq!(
            parse(&args(&["serve", "-"])),
            Err(CliError::UnexpectedArgument("-".into()))
        );
        assert_eq!(
            parse(&args(&["serve", "--", "x"])),
            Err(CliError::UnexpectedArgument("x".into()))
        );
    }
}
