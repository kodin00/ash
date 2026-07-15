use clap::{Parser, Subcommand};
use inquire::{Select, Text, error::InquireError, validator::Validation};
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

#[derive(Parser, Debug)]
#[command(
    name = "ash",
    version,
    about = "Connect to saved SSH machines without remembering hosts or key paths",
    after_help = "CONNECT:\n    ash <alias> [ssh options]",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Save a machine
    Add {
        /// Short name used by `ash <alias>`
        alias: Option<String>,
        /// SSH destination in USER:HOST form
        target: Option<String>,
        /// Optional private key; omit it to use the normal SSH password/agent flow
        identity_file: Option<PathBuf>,
        /// Replace an existing machine with the same alias
        #[arg(long, short)]
        force: bool,
    },
    /// List saved machines
    List,
    /// Remove a saved machine
    Remove {
        /// Machine alias to remove
        alias: String,
    },
    /// Treat any other command as a machine alias and connect with SSH
    #[command(external_subcommand)]
    Connect(Vec<OsString>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Machine {
    alias: String,
    user: String,
    host: String,
    identity_file: Option<PathBuf>,
}

#[derive(Default, Debug)]
struct Config {
    machines: Vec<Machine>,
}

#[derive(Debug)]
struct AppError(String);

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<io::Error> for AppError {
    fn from(error: io::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<InquireError> for AppError {
    fn from(error: InquireError) -> Self {
        match error {
            InquireError::OperationCanceled => Self("interactive add was cancelled".into()),
            InquireError::OperationInterrupted => Self("interactive add was interrupted".into()),
            InquireError::NotTTY => Self(
                "interactive add requires a terminal; pass <alias> <user>:<host> instead".into(),
            ),
            other => Self(format!("interactive prompt failed: {other}")),
        }
    }
}

type Result<T> = std::result::Result<T, AppError>;

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("ash: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<u8> {
    let path = config_path()?;

    match cli.command {
        Commands::Add {
            alias,
            target,
            identity_file,
            force,
        } => {
            let (alias, user, host, identity_file) =
                collect_machine_details(alias, target, identity_file)?;
            validate_alias(&alias)?;
            let mut config = Config::load(&path)?;

            if let Some(existing) = config.machines.iter_mut().find(|item| item.alias == alias) {
                if !force {
                    return Err(AppError(format!(
                        "alias '{alias}' already exists; pass --force to replace it"
                    )));
                }
                *existing = Machine {
                    alias: alias.clone(),
                    user,
                    host,
                    identity_file,
                };
            } else {
                config.machines.push(Machine {
                    alias: alias.clone(),
                    user,
                    host,
                    identity_file,
                });
            }

            config.save(&path)?;
            println!("Saved '{alias}'. Connect with: ash {alias}");
            Ok(0)
        }
        Commands::List => {
            let mut config = Config::load(&path)?;
            config.machines.sort_by(|a, b| a.alias.cmp(&b.alias));
            print_machines(&config.machines);
            Ok(0)
        }
        Commands::Remove { alias } => {
            let mut config = Config::load(&path)?;
            let old_len = config.machines.len();
            config.machines.retain(|machine| machine.alias != alias);
            if config.machines.len() == old_len {
                return Err(AppError(format!("unknown machine alias '{alias}'")));
            }
            config.save(&path)?;
            println!("Removed '{alias}'.");
            Ok(0)
        }
        Commands::Connect(args) => connect(&path, args),
    }
}

#[derive(Clone, Debug)]
enum KeyChoice {
    PasswordOrAgent,
    PrivateKey(PathBuf),
    CustomPath,
}

impl fmt::Display for KeyChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PasswordOrAgent => f.write_str("Password / SSH agent (no key file)"),
            Self::PrivateKey(path) => write!(f, "Private key: {}", display_home_path(path)),
            Self::CustomPath => f.write_str("Enter another private-key path"),
        }
    }
}

fn collect_machine_details(
    alias: Option<String>,
    target: Option<String>,
    identity_file: Option<PathBuf>,
) -> Result<(String, String, String, Option<PathBuf>)> {
    if let Some(target) = target {
        let alias = alias.ok_or_else(|| AppError("machine alias is required".into()))?;
        let (user, host) = parse_target(&target)?;
        let identity_file = identity_file.map(expand_path).transpose()?;
        return Ok((alias, user, host, identity_file));
    }

    if identity_file.is_some() {
        return Err(AppError(
            "an identity file can only follow an alias and USER:HOST target".into(),
        ));
    }

    let alias = match alias {
        Some(alias) => alias,
        None => Text::new("Machine alias:")
            .with_placeholder("machine-a")
            .with_validator(|value: &str| match validate_alias(value) {
                Ok(()) => Ok(Validation::Valid),
                Err(error) => Ok(Validation::Invalid(error.to_string().into())),
            })
            .prompt()?,
    };
    validate_alias(&alias)?;

    let user = Text::new("SSH user:")
        .with_placeholder("root")
        .with_validator(non_empty_no_whitespace_validator("user"))
        .prompt()?;
    let host = Text::new("IP address or hostname:")
        .with_placeholder("192.168.1.41")
        .with_validator(non_empty_no_whitespace_validator("host"))
        .prompt()?;
    let identity_file = prompt_for_identity_file()?;

    Ok((alias, user, host, identity_file))
}

fn non_empty_no_whitespace_validator(
    field: &'static str,
) -> impl Fn(&str) -> std::result::Result<Validation, inquire::CustomUserError> + Clone {
    move |value: &str| {
        if value.is_empty() {
            Ok(Validation::Invalid(
                format!("{field} cannot be empty").into(),
            ))
        } else if value.chars().any(char::is_whitespace) {
            Ok(Validation::Invalid(
                format!("{field} cannot contain whitespace").into(),
            ))
        } else {
            Ok(Validation::Valid)
        }
    }
}

fn prompt_for_identity_file() -> Result<Option<PathBuf>> {
    let mut choices = vec![KeyChoice::PasswordOrAgent];
    choices.extend(
        discover_private_keys()?
            .into_iter()
            .map(KeyChoice::PrivateKey),
    );
    choices.push(KeyChoice::CustomPath);

    let choice = Select::new("SSH authentication:", choices)
        .with_help_message("Use the arrow keys, then press Enter")
        .with_page_size(10)
        .prompt()?;

    match choice {
        KeyChoice::PasswordOrAgent => Ok(None),
        KeyChoice::PrivateKey(path) => Ok(Some(expand_path(path)?)),
        KeyChoice::CustomPath => {
            let path = Text::new("Private-key path:")
                .with_placeholder("~/.ssh/id_ed25519")
                .prompt()?;
            Ok(Some(expand_path(PathBuf::from(path))?))
        }
    }
}

fn discover_private_keys() -> Result<Vec<PathBuf>> {
    let Some(home) = env::var_os("HOME") else {
        return Ok(Vec::new());
    };
    discover_private_keys_in(&PathBuf::from(home).join(".ssh"))
}

fn discover_private_keys_in(directory: &Path) -> Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(AppError(format!(
                "could not scan {} for SSH keys: {error}",
                directory.display()
            )));
        }
    };

    let mut keys = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if looks_like_private_key(&path) {
            keys.push(path);
        }
    }
    keys.sort();
    Ok(keys)
}

fn looks_like_private_key(path: &Path) -> bool {
    if path.extension().is_some_and(|extension| extension == "pub") {
        return false;
    }
    let Ok(file) = File::open(path) else {
        return false;
    };
    let mut first_line = String::new();
    if BufReader::new(file).read_line(&mut first_line).is_err() {
        return false;
    }
    let header = first_line.trim();
    (header.starts_with("-----BEGIN ") && header.ends_with(" PRIVATE KEY-----"))
        || header.starts_with("PuTTY-User-Key-File-")
}

fn display_home_path(path: &Path) -> String {
    if let Some(home) = env::var_os("HOME")
        && let Ok(relative) = path.strip_prefix(PathBuf::from(home))
    {
        return Path::new("~").join(relative).display().to_string();
    }
    path.display().to_string()
}

impl Config {
    fn load(path: &Path) -> Result<Self> {
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => {
                return Err(AppError(format!(
                    "could not read {}: {error}",
                    path.display()
                )));
            }
        };

        let mut machines = Vec::new();
        for (index, line) in contents.lines().enumerate() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            if !(fields.len() == 3 || fields.len() == 4) {
                return Err(AppError(format!(
                    "invalid config at {}:{} (expected 3 or 4 tab-separated fields)",
                    path.display(),
                    index + 1
                )));
            }
            let identity_file = fields
                .get(3)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from);
            machines.push(Machine {
                alias: fields[0].to_owned(),
                user: fields[1].to_owned(),
                host: fields[2].to_owned(),
                identity_file,
            });
        }
        Ok(Self { machines })
    }

    fn save(&mut self, path: &Path) -> Result<()> {
        self.machines.sort_by(|a, b| a.alias.cmp(&b.alias));
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| {
                AppError(format!("could not create {}: {error}", parent.display()))
            })?;
        }

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary = path.with_extension(format!("tmp-{}-{nonce}", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);

        let write_result = (|| -> Result<()> {
            let mut file = options.open(&temporary).map_err(|error| {
                AppError(format!("could not create {}: {error}", temporary.display()))
            })?;
            writeln!(
                file,
                "# ash config v1: alias<TAB>user<TAB>host<TAB>identity_file"
            )?;
            for machine in &self.machines {
                validate_field(&machine.alias)?;
                validate_field(&machine.user)?;
                validate_field(&machine.host)?;
                let identity = machine
                    .identity_file
                    .as_deref()
                    .map(|path| path.to_string_lossy())
                    .unwrap_or_default();
                validate_field(&identity)?;
                writeln!(
                    file,
                    "{}\t{}\t{}\t{}",
                    machine.alias, machine.user, machine.host, identity
                )?;
            }
            file.sync_all()?;
            fs::rename(&temporary, path).map_err(|error| {
                AppError(format!("could not replace {}: {error}", path.display()))
            })?;
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result
    }
}

fn config_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("ASH_CONFIG_FILE") {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path).join("ash/config"));
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join(".config/ash/config"))
        .ok_or_else(|| {
            AppError("HOME is not set; set ASH_CONFIG_FILE to choose a config path".into())
        })
}

fn validate_alias(alias: &str) -> Result<()> {
    if alias.is_empty()
        || !alias
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        return Err(AppError(
            "aliases may contain only ASCII letters, numbers, '.', '_' and '-'".into(),
        ));
    }
    if matches!(alias, "add" | "list" | "remove" | "help") {
        return Err(AppError(format!("'{alias}' is reserved by ash")));
    }
    Ok(())
}

fn validate_field(value: &str) -> Result<()> {
    if value.contains(['\t', '\n', '\r']) {
        return Err(AppError(
            "config values cannot contain tabs or newlines".into(),
        ));
    }
    Ok(())
}

fn parse_target(target: &str) -> Result<(String, String)> {
    let (user, host) = target.split_once(':').ok_or_else(|| {
        AppError("target must be in USER:HOST form, for example root:192.168.1.20".into())
    })?;
    if user.is_empty() || host.is_empty() || user.chars().any(char::is_whitespace) {
        return Err(AppError(
            "target must contain a non-empty user and host".into(),
        ));
    }
    if host.chars().any(char::is_whitespace) {
        return Err(AppError("host cannot contain whitespace".into()));
    }
    Ok((user.to_owned(), host.to_owned()))
}

fn expand_path(path: PathBuf) -> Result<PathBuf> {
    let expanded = if path == Path::new("~") {
        env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| AppError("cannot expand '~' because HOME is not set".into()))?
    } else if let Ok(rest) = path.strip_prefix("~/") {
        env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| AppError("cannot expand '~' because HOME is not set".into()))?
            .join(rest)
    } else {
        path
    };

    let absolute = fs::canonicalize(&expanded).map_err(|error| {
        AppError(format!(
            "identity file {} is not accessible: {error}",
            expanded.display()
        ))
    })?;
    if !absolute.is_file() {
        return Err(AppError(format!(
            "identity file {} is not a regular file",
            absolute.display()
        )));
    }
    Ok(absolute)
}

fn print_machines(machines: &[Machine]) {
    if machines.is_empty() {
        println!("No machines saved. Add one with: ash add <alias> <user>:<host> [key-file]");
        return;
    }

    println!("{:<20} {:<28} IDENTITY", "ALIAS", "DESTINATION");
    for machine in machines {
        let destination = format!("{}@{}", machine.user, machine.host);
        let identity = machine
            .identity_file
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "password / SSH agent".into());
        println!("{:<20} {:<28} {}", machine.alias, destination, identity);
    }
}

fn connect(path: &Path, args: Vec<OsString>) -> Result<u8> {
    let alias = args
        .first()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError("machine alias must be valid UTF-8".into()))?;
    let config = Config::load(path)?;
    let machine = config
        .machines
        .iter()
        .find(|machine| machine.alias == alias)
        .ok_or_else(|| {
            AppError(format!(
                "unknown machine alias '{alias}'; run `ash list` to see saved machines"
            ))
        })?;

    let mut command = ssh_command(machine, &args[1..]);

    #[cfg(unix)]
    {
        let error = command.exec();
        Err(AppError(format!("could not launch ssh: {error}")))
    }

    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .map_err(|error| AppError(format!("could not launch ssh: {error}")))?;
        Ok(status.code().unwrap_or(1).clamp(0, 255) as u8)
    }
}

fn ssh_command(machine: &Machine, extra_args: &[OsString]) -> Command {
    let mut command = Command::new("ssh");
    if let Some(identity_file) = &machine.identity_file {
        command.arg("-i").arg(identity_file);
    }
    command.args(extra_args);
    command.arg(format!("{}@{}", machine.user, machine.host));
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ipv4_and_ipv6_targets() {
        assert_eq!(
            parse_target("root:192.168.1.4").unwrap(),
            ("root".into(), "192.168.1.4".into())
        );
        assert_eq!(
            parse_target("ubuntu:2001:db8::1").unwrap(),
            ("ubuntu".into(), "2001:db8::1".into())
        );
    }

    #[test]
    fn rejects_bad_aliases_and_targets() {
        assert!(validate_alias("my machine").is_err());
        assert!(validate_alias("list").is_err());
        assert!(parse_target("root@host").is_err());
        assert!(parse_target(":host").is_err());
    }

    #[test]
    fn clap_captures_machine_alias_and_ssh_options() {
        let cli = Cli::try_parse_from(["ash", "web-1", "-L", "8080:localhost:80"]).unwrap();
        match cli.command {
            Commands::Connect(args) => {
                assert_eq!(args, ["web-1", "-L", "8080:localhost:80"]);
            }
            _ => panic!("expected external connect command"),
        }
    }

    #[test]
    fn clap_accepts_add_without_arguments() {
        let cli = Cli::try_parse_from(["ash", "add"]).unwrap();
        match cli.command {
            Commands::Add {
                alias,
                target,
                identity_file,
                force,
            } => {
                assert_eq!(alias, None);
                assert_eq!(target, None);
                assert_eq!(identity_file, None);
                assert!(!force);
            }
            _ => panic!("expected add command"),
        }
    }

    #[test]
    fn config_round_trip() {
        let path = env::temp_dir().join(format!(
            "ash-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut config = Config {
            machines: vec![Machine {
                alias: "box".into(),
                user: "root".into(),
                host: "10.0.0.2".into(),
                identity_file: Some(PathBuf::from("/tmp/example-key")),
            }],
        };
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().machines, config.machines);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn builds_expected_ssh_command() {
        let machine = Machine {
            alias: "web".into(),
            user: "deploy".into(),
            host: "10.0.0.8".into(),
            identity_file: Some(PathBuf::from("/keys/web")),
        };
        let extra = [OsString::from("-p"), OsString::from("2222")];
        let command = ssh_command(&machine, &extra);
        assert_eq!(command.get_program(), "ssh");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["-i", "/keys/web", "-p", "2222", "deploy@10.0.0.8"]
        );
    }

    #[test]
    fn discovers_private_keys_but_not_public_or_config_files() {
        let directory = env::temp_dir().join(format!(
            "ash-key-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let ed25519 = directory.join("id_ed25519");
        let rsa = directory.join("work_rsa");
        fs::write(&ed25519, "-----BEGIN OPENSSH PRIVATE KEY-----\ndata").unwrap();
        fs::write(&rsa, "-----BEGIN RSA PRIVATE KEY-----\ndata").unwrap();
        fs::write(directory.join("id_ed25519.pub"), "ssh-ed25519 public").unwrap();
        fs::write(directory.join("config"), "Host example").unwrap();

        assert_eq!(
            discover_private_keys_in(&directory).unwrap(),
            [ed25519, rsa]
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
