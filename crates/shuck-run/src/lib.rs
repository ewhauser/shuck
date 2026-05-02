use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tempfile::Builder as TempFileBuilder;
use url::Url;

const DEFAULT_REGISTRY_URL: &str = "https://shells.shuck.dev/index.json";
const REGISTRY_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const SHELLS_DIR_ENV: &str = "SHUCK_SHELLS_DIR";
const REGISTRY_URL_ENV: &str = "SHUCK_RUN_REGISTRY_URL";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Shell {
    Bash,
    Zsh,
    Dash,
    Mksh,
}

impl Shell {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Dash => "dash",
            Self::Mksh => "mksh",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "dash" | "sh" => Some(Self::Dash),
            "mksh" | "ksh" => Some(Self::Mksh),
            _ => None,
        }
    }

    fn infer(source: &str, path: Option<&Path>) -> Option<Self> {
        Self::infer_from_shebang(source).or_else(|| {
            path.and_then(|path| {
                let ext = path.extension()?.to_str()?;
                Self::from_name(ext)
            })
        })
    }

    fn infer_from_shebang(source: &str) -> Option<Self> {
        let interpreter = shuck_parser::shebang::interpreter_name(source.lines().next()?)?;
        Self::from_name(interpreter)
    }
}

impl fmt::Display for Shell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    raw: String,
    tokens: Vec<VersionToken>,
    segment_count: usize,
    prefix_match: bool,
}

impl Version {
    pub fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            bail!("version cannot be empty");
        }

        let tokens = tokenize_version(raw);
        if tokens.is_empty() {
            bail!("invalid version `{raw}`");
        }

        let segment_count = raw.split('.').filter(|segment| !segment.is_empty()).count();
        let prefix_match = should_treat_as_prefix(raw, &tokens, segment_count);

        Ok(Self {
            raw: raw.to_owned(),
            tokens,
            segment_count,
            prefix_match,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    fn matches_prefix(&self, other: &Self) -> bool {
        if !self.prefix_match {
            return self == other;
        }

        let mut matched_segments = 0usize;
        let mut left = self.tokens.iter().peekable();
        let mut right = other.tokens.iter().peekable();
        while let Some(left_token) = left.next() {
            let Some(right_token) = right.next() else {
                return false;
            };
            if left_token != right_token {
                return false;
            }
            if matches!(left_token, VersionToken::Numeric(_) | VersionToken::Text(_))
                && (left.peek().is_none()
                    || matches!(
                        left.peek(),
                        Some(VersionToken::Numeric(_) | VersionToken::Text(_))
                    ))
            {
                matched_segments += 1;
            }
        }

        matched_segments >= self.segment_count
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let mut left = self.tokens.iter();
        let mut right = other.tokens.iter();
        loop {
            match (left.next(), right.next()) {
                (Some(a), Some(b)) => {
                    let ordering = a.cmp(b);
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                }
                (Some(_), None) => return Ordering::Greater,
                (None, Some(_)) => return Ordering::Less,
                (None, None) => return Ordering::Equal,
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum VersionToken {
    Numeric(u64),
    Text(String),
}

fn tokenize_version(raw: &str) -> Vec<VersionToken> {
    let mut tokens = Vec::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.peek().copied() {
        if ch.is_ascii_digit() {
            let mut digits = String::new();
            while let Some(next) = chars.peek().copied() {
                if next.is_ascii_digit() {
                    digits.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            if let Ok(value) = digits.parse::<u64>() {
                tokens.push(VersionToken::Numeric(value));
            }
            continue;
        }

        if ch.is_ascii_alphabetic() {
            let mut text = String::new();
            while let Some(next) = chars.peek().copied() {
                if next.is_ascii_alphabetic() {
                    text.push(next.to_ascii_lowercase());
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(VersionToken::Text(text));
            continue;
        }

        if ch == '.' {
            chars.next();
            continue;
        }

        break;
    }

    tokens
}

fn should_treat_as_prefix(raw: &str, tokens: &[VersionToken], segment_count: usize) -> bool {
    raw.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
        && segment_count == 2
        && tokens
            .iter()
            .all(|token| matches!(token, VersionToken::Numeric(_)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionConstraint {
    Latest,
    Exact(Version),
    ExactPrefix(Version),
    Range(Vec<VersionPredicate>),
}

impl VersionConstraint {
    pub fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        if raw.eq_ignore_ascii_case("latest") {
            return Ok(Self::Latest);
        }

        if raw.contains('>') || raw.contains('<') || raw.contains('=') {
            let mut predicates = Vec::new();
            for item in raw.split(',') {
                let item = item.trim();
                if item.is_empty() {
                    bail!("invalid version constraint `{raw}`");
                }
                predicates.push(VersionPredicate::parse(item)?);
            }
            return Ok(Self::Range(predicates));
        }

        let version = Version::parse(raw)?;
        if version.prefix_match {
            Ok(Self::ExactPrefix(version))
        } else {
            Ok(Self::Exact(version))
        }
    }

    pub fn matches(&self, version: &Version) -> bool {
        match self {
            Self::Latest => true,
            Self::Exact(expected) => expected == version,
            Self::ExactPrefix(prefix) => prefix.matches_prefix(version),
            Self::Range(predicates) => predicates
                .iter()
                .all(|predicate| predicate.matches(version)),
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Latest => "latest".to_owned(),
            Self::Exact(version) | Self::ExactPrefix(version) => version.as_str().to_owned(),
            Self::Range(predicates) => predicates
                .iter()
                .map(VersionPredicate::to_string)
                .collect::<Vec<_>>()
                .join(","),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableShell {
    pub shell: Shell,
    pub versions: Vec<Version>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct RunConfig {
    pub shell: Option<String>,
    pub shell_version: Option<String>,
    pub shells: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInterpreter {
    pub shell: Shell,
    pub version: Version,
    pub path: PathBuf,
    pub source: ResolutionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionSource {
    Managed,
    System,
}

#[derive(Debug, Clone)]
pub struct ResolveOptions<'a> {
    pub shell: Option<Shell>,
    pub version: Option<VersionConstraint>,
    pub system: bool,
    pub script: Option<&'a Path>,
    pub config: Option<&'a RunConfig>,
    pub verbose: bool,
    pub refresh_registry: bool,
}

impl<'a> ResolveOptions<'a> {
    pub fn new(
        shell: Option<Shell>,
        version: Option<VersionConstraint>,
        system: bool,
        script: Option<&'a Path>,
        config: Option<&'a RunConfig>,
    ) -> Self {
        Self {
            shell,
            version,
            system,
            script,
            config,
            verbose: false,
            refresh_registry: false,
        }
    }
}

pub fn resolve(
    shell: Option<Shell>,
    version: Option<VersionConstraint>,
    system: bool,
    script: Option<&Path>,
    config: Option<&RunConfig>,
) -> Result<ResolvedInterpreter> {
    resolve_with_options(ResolveOptions::new(shell, version, system, script, config))
}

pub fn resolve_with_options(options: ResolveOptions<'_>) -> Result<ResolvedInterpreter> {
    let environment = Environment::from_process()?;
    resolve_with_environment(&environment, options)
}

pub fn install(shell: Shell, version: &VersionConstraint) -> Result<ResolvedInterpreter> {
    install_with_options(shell, version, false, false)
}

pub fn install_with_options(
    shell: Shell,
    version: &VersionConstraint,
    verbose: bool,
    refresh_registry: bool,
) -> Result<ResolvedInterpreter> {
    let environment = Environment::from_process()?;
    install_with_environment(&environment, shell, version, verbose, refresh_registry)
}

pub fn list_available(shell: Option<Shell>) -> Result<Vec<AvailableShell>> {
    list_available_with_options(shell, false, false)
}

pub fn list_available_with_options(
    shell: Option<Shell>,
    refresh_registry: bool,
    verbose: bool,
) -> Result<Vec<AvailableShell>> {
    let environment = Environment::from_process()?;
    let registry = load_registry(&environment, refresh_registry, verbose)?;
    Ok(available_shells(&registry, shell))
}

fn resolve_with_environment(
    environment: &Environment,
    options: ResolveOptions<'_>,
) -> Result<ResolvedInterpreter> {
    let script_info = options
        .script
        .map(read_script_info)
        .transpose()?
        .unwrap_or_default();

    let config_shell = shell_from_config(options.config)?;
    let shell = options
        .shell
        .or(script_info.metadata.as_ref().map(|metadata| metadata.shell))
        .or(config_shell)
        .or(script_info.inferred_shell)
        .ok_or_else(|| anyhow!("Cannot determine shell. Specify --shell or add a shebang."))?;

    let version = if let Some(version) = options.version {
        version
    } else if let Some(version) = script_info
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.version.clone())
    {
        version
    } else if let Some(version) = config_version_for_shell(options.config, shell)? {
        version
    } else {
        VersionConstraint::Latest
    };

    if options.system {
        resolve_system(shell, &version)
    } else {
        install_with_environment(
            environment,
            shell,
            &version,
            options.verbose,
            options.refresh_registry,
        )
    }
}

fn shell_from_config(config: Option<&RunConfig>) -> Result<Option<Shell>> {
    let Some(config) = config else {
        return Ok(None);
    };
    config.shell.as_deref().map(parse_shell_name).transpose()
}

fn config_version_for_shell(
    config: Option<&RunConfig>,
    shell: Shell,
) -> Result<Option<VersionConstraint>> {
    let Some(config) = config else {
        return Ok(None);
    };

    if let Some(raw) = config.shells.get(shell.as_str()) {
        return Ok(Some(VersionConstraint::parse(raw)?));
    }

    config
        .shell_version
        .as_deref()
        .map(VersionConstraint::parse)
        .transpose()
}

fn parse_shell_name(raw: &str) -> Result<Shell> {
    Shell::from_name(raw)
        .ok_or_else(|| anyhow!("unsupported shell `{raw}`; expected one of: bash, zsh, dash, mksh"))
}

fn resolve_system(shell: Shell, constraint: &VersionConstraint) -> Result<ResolvedInterpreter> {
    let path = find_on_path(shell.as_str())
        .ok_or_else(|| anyhow!("{shell} not found on $PATH. Install it or remove --system."))?;
    resolve_system_at_path(shell, &path, constraint)
}

fn resolve_system_at_path(
    shell: Shell,
    path: &Path,
    constraint: &VersionConstraint,
) -> Result<ResolvedInterpreter> {
    let version = detect_shell_version(shell, path)
        .with_context(|| format!("detect system {shell} version at {}", path.display()))?;
    if !constraint.matches(&version) {
        bail!(
            "System {shell} is {}, but {} is required. Run `shuck install {shell} {}` to get a managed version.",
            version,
            constraint.describe(),
            constraint.describe()
        );
    }

    Ok(ResolvedInterpreter {
        shell,
        version,
        path: path.to_path_buf(),
        source: ResolutionSource::System,
    })
}

fn install_with_environment(
    environment: &Environment,
    shell: Shell,
    constraint: &VersionConstraint,
    verbose: bool,
    refresh_registry: bool,
) -> Result<ResolvedInterpreter> {
    let registry = load_registry(environment, refresh_registry, verbose)?;
    let platform = current_platform()?;
    let version = select_version(&registry, shell, constraint)?;
    let artifact = registry
        .shells
        .get(shell.as_str())
        .and_then(|entry| entry.versions.get(version.as_str()))
        .and_then(|entry| entry.platforms.get(&platform))
        .ok_or_else(|| {
            anyhow!("{shell} {version} does not have a prebuilt binary for {platform}.")
        })?;

    let install_dir = environment
        .shells_root
        .join(shell.as_str())
        .join(version.as_str())
        .join(&platform);
    let binary_path = install_dir.join("bin").join(shell.as_str());
    if binary_path.exists() {
        let detected = detect_shell_version(shell, &binary_path)
            .with_context(|| format!("verify {}", binary_path.display()))?;
        if detected != version {
            bail!("installed {shell} reports version {detected}, expected {version}");
        }
        return Ok(ResolvedInterpreter {
            shell,
            version,
            path: binary_path,
            source: ResolutionSource::Managed,
        });
    }

    fs::create_dir_all(&environment.shells_root)
        .with_context(|| format!("create {}", environment.shells_root.display()))?;
    let archive = TempFileBuilder::new()
        .prefix("shuck-shell-")
        .suffix(".tar.gz")
        .tempfile_in(&environment.shells_root)
        .with_context(|| {
            format!(
                "create temp archive in {}",
                environment.shells_root.display()
            )
        })?;
    fetch_url_to_path(&artifact.url, archive.path(), verbose)
        .with_context(|| format!("download {shell} {version}"))?;
    verify_sha256(archive.path(), &artifact.sha256)
        .with_context(|| format!("verify checksum for {shell} {version}"))?;

    let tempdir = TempFileBuilder::new()
        .prefix("shuck-install-")
        .tempdir_in(&environment.shells_root)
        .with_context(|| {
            format!(
                "create temp install dir in {}",
                environment.shells_root.display()
            )
        })?;
    extract_archive(archive.path(), tempdir.path())
        .with_context(|| format!("extract {}", archive.path().display()))?;

    let extracted_root = locate_extracted_root(tempdir.path(), shell)
        .ok_or_else(|| anyhow!("archive did not contain bin/{}", shell.as_str()))?;
    fs::create_dir_all(
        install_dir
            .parent()
            .ok_or_else(|| anyhow!("invalid install directory {}", install_dir.display()))?,
    )?;

    if install_dir.exists() {
        return Ok(ResolvedInterpreter {
            shell,
            version,
            path: binary_path,
            source: ResolutionSource::Managed,
        });
    }

    if extracted_root == tempdir.path() {
        let temp_path = tempdir.keep();
        fs::rename(&temp_path, &install_dir).or_else(|err| {
            if install_dir.exists() {
                Ok(())
            } else {
                Err(err)
            }
        })?;
    } else {
        fs::rename(&extracted_root, &install_dir).or_else(|err| {
            if install_dir.exists() {
                Ok(())
            } else {
                Err(err)
            }
        })?;
    }

    let binary_path = install_dir.join("bin").join(shell.as_str());
    let detected = detect_shell_version(shell, &binary_path)
        .with_context(|| format!("verify {}", binary_path.display()))?;
    if detected != version {
        bail!("installed {shell} reports version {detected}, expected {version}");
    }

    Ok(ResolvedInterpreter {
        shell,
        version,
        path: binary_path,
        source: ResolutionSource::Managed,
    })
}

fn available_shells(registry: &RegistryIndex, shell: Option<Shell>) -> Vec<AvailableShell> {
    let shells = shell.into_iter().collect::<Vec<_>>();
    let names = if shells.is_empty() {
        registry
            .shells
            .keys()
            .filter_map(|name| Shell::from_name(name))
            .collect::<Vec<_>>()
    } else {
        shells
    };

    let mut available = names
        .into_iter()
        .filter_map(|shell| {
            registry.shells.get(shell.as_str()).map(|entry| {
                let mut versions = entry
                    .versions
                    .keys()
                    .filter_map(|version| Version::parse(version).ok())
                    .collect::<Vec<_>>();
                versions.sort();
                versions.reverse();
                AvailableShell { shell, versions }
            })
        })
        .collect::<Vec<_>>();
    available.sort_by_key(|entry| entry.shell);
    available
}

fn select_version(
    registry: &RegistryIndex,
    shell: Shell,
    constraint: &VersionConstraint,
) -> Result<Version> {
    let shell_entry = registry.shells.get(shell.as_str()).ok_or_else(|| {
        anyhow!(
            "{} is not available. Run `shuck install --list` to see available shells.",
            shell
        )
    })?;

    let mut versions = shell_entry
        .versions
        .keys()
        .map(|version| Version::parse(version))
        .collect::<Result<Vec<_>>>()?;
    versions.sort();

    versions
        .into_iter()
        .rev()
        .find(|version| constraint.matches(version))
        .ok_or_else(|| {
            anyhow!(
                "{shell} {} is not available. Run `shuck install --list {shell}` to see available versions.",
                constraint.describe()
            )
        })
}

fn load_registry(environment: &Environment, refresh: bool, verbose: bool) -> Result<RegistryIndex> {
    fs::create_dir_all(&environment.shells_root)
        .with_context(|| format!("create {}", environment.shells_root.display()))?;
    let index_path = environment.shells_root.join("index.json");

    let should_refresh =
        refresh || !index_path.exists() || index_is_stale(&index_path).unwrap_or(true);
    if should_refresh {
        match fetch_url_to_path(&environment.registry_url, &index_path, verbose) {
            Ok(()) => {}
            Err(_err) if index_path.exists() && !refresh => {}
            Err(err) => {
                return Err(err).with_context(|| format!("fetch {}", environment.registry_url));
            }
        }
    }

    let source = fs::read_to_string(&index_path)
        .with_context(|| format!("read {}", index_path.display()))?;
    serde_json::from_str(&source).with_context(|| format!("parse {}", index_path.display()))
}

fn index_is_stale(path: &Path) -> Result<bool> {
    let modified = fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .modified()
        .with_context(|| format!("read mtime for {}", path.display()))?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    Ok(age > REGISTRY_MAX_AGE)
}

fn current_platform() -> Result<String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("x86_64-linux".to_owned()),
        ("linux", "aarch64") => Ok("aarch64-linux".to_owned()),
        ("macos", "x86_64") => Ok("x86_64-darwin".to_owned()),
        ("macos", "aarch64") => Ok("aarch64-darwin".to_owned()),
        (os, arch) => bail!("unsupported platform {arch}-{os}"),
    }
}

fn fetch_url_to_path(url: &str, dest: &Path, verbose: bool) -> Result<()> {
    if let Some(source_path) = file_url_path(url)? {
        fs::copy(&source_path, dest)
            .with_context(|| format!("copy {} to {}", source_path.display(), dest.display()))?;
        return Ok(());
    }

    let mut command = Command::new("curl");
    command.arg("--fail").arg("--location");
    command.arg("--retry").arg("1").arg("--retry-all-errors");
    if !verbose {
        command.arg("--silent").arg("--show-error");
    }
    command.arg("--output").arg(dest);
    command.arg(url);
    let status = command.status().context("run curl")?;
    if !status.success() {
        bail!("curl failed while fetching {url}");
    }
    Ok(())
}

fn file_url_path(raw_url: &str) -> Result<Option<PathBuf>> {
    let Ok(parsed_url) = Url::parse(raw_url) else {
        return Ok(None);
    };
    if parsed_url.scheme() != "file" {
        return Ok(None);
    }
    parsed_url
        .to_file_path()
        .map(Some)
        .map_err(|_| anyhow!("invalid file URL `{raw_url}`"))
}

fn extract_archive(archive: &Path, destination: &Path) -> Result<()> {
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .status()
        .context("run tar")?;
    if !status.success() {
        bail!("tar failed while extracting {}", archive.display());
    }
    Ok(())
}

fn locate_extracted_root(root: &Path, shell: Shell) -> Option<PathBuf> {
    let direct = root.join("bin").join(shell.as_str());
    if direct.exists() {
        return Some(root.to_path_buf());
    }

    fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join("bin").join(shell.as_str()).exists())
}

fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let digest = Sha256::digest(bytes);
    let actual = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual != expected.trim().to_ascii_lowercase() {
        bail!(
            "Checksum mismatch. Expected {}, got {}. The archive may be corrupted.",
            expected,
            actual
        );
    }
    Ok(())
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var).find_map(|directory| {
        let candidate = directory.join(name);
        candidate.is_file().then_some(candidate)
    })
}

fn detect_shell_version(shell: Shell, path: &Path) -> Result<Version> {
    let candidates = version_probe_commands(shell);
    let mut last_error = None;
    for args in candidates {
        let output = Command::new(path).args(args).output();
        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = if stdout.trim().is_empty() {
                    stderr.trim().to_owned()
                } else if stderr.trim().is_empty() {
                    stdout.trim().to_owned()
                } else {
                    format!("{}\n{}", stdout.trim(), stderr.trim())
                };
                if let Some(version) = extract_version(shell, &combined) {
                    return Ok(version);
                }
                last_error = Some(anyhow!(
                    "could not parse version from `{}`",
                    combined.trim()
                ));
            }
            Err(err) => last_error = Some(err.into()),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("unable to detect shell version")))
}

fn version_probe_commands(shell: Shell) -> Vec<Vec<OsString>> {
    match shell {
        Shell::Bash | Shell::Zsh => vec![vec![OsString::from("--version")]],
        Shell::Dash => vec![
            vec![OsString::from("-V")],
            vec![OsString::from("--version")],
        ],
        Shell::Mksh => vec![
            vec![
                OsString::from("-c"),
                OsString::from("printf '%s' \"$KSH_VERSION\""),
            ],
            vec![OsString::from("-V")],
        ],
    }
}

fn extract_version(shell: Shell, text: &str) -> Option<Version> {
    if shell == Shell::Mksh
        && let Some(version) = extract_mksh_version(text)
    {
        return Version::parse(&version).ok();
    }

    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.peek().copied() {
        if !ch.is_ascii_digit() {
            chars.next();
            continue;
        }

        let mut candidate = String::new();
        while let Some(next) = chars.peek().copied() {
            if next.is_ascii_digit() || next == '.' || next.is_ascii_alphabetic() {
                candidate.push(next);
                chars.next();
            } else {
                break;
            }
        }

        if candidate.chars().any(|ch| ch.is_ascii_digit())
            && let Ok(version) = Version::parse(&candidate)
        {
            return Some(version);
        }
    }

    None
}

fn extract_mksh_version(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    for index in 0..bytes.len().saturating_sub(1) {
        if bytes[index] == b'R' && bytes[index + 1].is_ascii_digit() {
            let mut end = index + 1;
            while end < bytes.len()
                && (bytes[end].is_ascii_digit() || bytes[end].is_ascii_alphabetic())
            {
                end += 1;
            }
            return Some(text[index + 1..end].to_owned());
        }
    }
    None
}

#[derive(Debug, Clone, Default)]
struct ScriptInfo {
    inferred_shell: Option<Shell>,
    metadata: Option<ScriptMetadata>,
}

fn read_script_info(path: &Path) -> Result<ScriptInfo> {
    let source = fs::read_to_string(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            anyhow!("{}: No such file", path.display())
        } else {
            anyhow!(err).context(format!("read {}", path.display()))
        }
    })?;
    Ok(ScriptInfo {
        inferred_shell: Shell::infer(&source, Some(path)),
        metadata: parse_script_metadata(&source)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScriptMetadata {
    shell: Shell,
    version: Option<VersionConstraint>,
}

fn parse_script_metadata(source: &str) -> Result<Option<ScriptMetadata>> {
    let mut start_line = None;
    let mut saw_body = false;
    for (line_index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(start) = trimmed.strip_prefix("# /// shuck") {
            if !start.trim().is_empty() {
                bail!("invalid shuck metadata header");
            }
            if saw_body {
                bail!("shuck metadata blocks must appear before the script body");
            }
            if start_line.is_some() {
                bail!("multiple `# /// shuck` blocks are not allowed");
            }
            start_line = Some(line_index);
            break;
        }

        if trimmed.starts_with('#') {
            continue;
        }

        saw_body = true;
    }

    let Some(start_line) = start_line else {
        return Ok(None);
    };

    let mut body = String::new();
    let mut lines = source.lines().enumerate().skip(start_line + 1);
    for (_, line) in lines.by_ref() {
        let trimmed = line.trim();
        if trimmed == "# ///" {
            let block: MetadataBlock = toml::from_str(&body).context("parse shuck metadata")?;
            let shell = parse_shell_name(&block.shell)?;
            let version = block
                .version
                .as_deref()
                .map(VersionConstraint::parse)
                .transpose()?;
            for (_, trailing_line) in lines {
                let trailing = trailing_line.trim();
                if trailing.is_empty() {
                    continue;
                }
                if trailing == "# /// shuck" {
                    bail!("multiple `# /// shuck` blocks are not allowed");
                }
                if trailing.starts_with('#') {
                    continue;
                }
                break;
            }
            return Ok(Some(ScriptMetadata { shell, version }));
        }

        let Some(comment_body) = line.trim_start().strip_prefix('#') else {
            bail!("shuck metadata block must stay in the leading comment header");
        };
        body.push_str(comment_body.strip_prefix(' ').unwrap_or(comment_body));
        body.push('\n');
    }

    bail!("unterminated `# /// shuck` metadata block")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataBlock {
    shell: String,
    version: Option<String>,
    #[serde(rename = "metadata")]
    _metadata: Option<BTreeMap<String, toml::Value>>,
}

#[derive(Debug, Deserialize)]
struct RegistryIndex {
    #[serde(rename = "version")]
    _version: u64,
    shells: BTreeMap<String, RegistryShell>,
}

#[derive(Debug, Deserialize)]
struct RegistryShell {
    versions: BTreeMap<String, RegistryVersion>,
}

#[derive(Debug, Deserialize)]
struct RegistryVersion {
    platforms: BTreeMap<String, RegistryArtifact>,
}

#[derive(Debug, Deserialize)]
struct RegistryArtifact {
    url: String,
    sha256: String,
}

#[derive(Debug, Clone)]
struct Environment {
    shells_root: PathBuf,
    registry_url: String,
}

impl Environment {
    fn from_process() -> Result<Self> {
        let shells_root = if let Some(value) = std::env::var_os(SHELLS_DIR_ENV) {
            PathBuf::from(value)
        } else {
            let home = etcetera::home_dir().context("resolve the home directory for shuck")?;
            home.join(".shuck").join("shells")
        };

        let registry_url =
            std::env::var(REGISTRY_URL_ENV).unwrap_or_else(|_| DEFAULT_REGISTRY_URL.to_owned());

        Ok(Self {
            shells_root,
            registry_url,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionPredicate {
    operator: VersionOperator,
    version: Version,
}

impl VersionPredicate {
    fn parse(raw: &str) -> Result<Self> {
        let (operator, version) = if let Some(version) = raw.strip_prefix(">=") {
            (VersionOperator::GreaterOrEqual, version)
        } else if let Some(version) = raw.strip_prefix("<=") {
            (VersionOperator::LessOrEqual, version)
        } else if let Some(version) = raw.strip_prefix('>') {
            (VersionOperator::Greater, version)
        } else if let Some(version) = raw.strip_prefix('<') {
            (VersionOperator::Less, version)
        } else if let Some(version) = raw.strip_prefix('=') {
            (VersionOperator::Equal, version)
        } else {
            bail!("invalid version predicate `{raw}`");
        };

        Ok(Self {
            operator,
            version: Version::parse(version)?,
        })
    }

    fn matches(&self, version: &Version) -> bool {
        match self.operator {
            VersionOperator::Greater => version > &self.version,
            VersionOperator::GreaterOrEqual => version >= &self.version,
            VersionOperator::Less => version < &self.version,
            VersionOperator::LessOrEqual => version <= &self.version,
            VersionOperator::Equal => version == &self.version,
        }
    }
}

impl fmt::Display for VersionPredicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.operator, self.version)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionOperator {
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
    Equal,
}

impl fmt::Display for VersionOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let symbol = match self {
            Self::Greater => ">",
            Self::GreaterOrEqual => ">=",
            Self::Less => "<",
            Self::LessOrEqual => "<=",
            Self::Equal => "=",
        };
        f.write_str(symbol)
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn test_environment(root: &Path, registry_url: String) -> Environment {
        Environment {
            shells_root: root.join("shells"),
            registry_url,
        }
    }

    fn write_registry(root: &Path, body: &str) -> PathBuf {
        let registry_path = root.join("registry.json");
        fs::write(&registry_path, body).unwrap();
        registry_path
    }

    fn make_shell_archive(root: &Path, shell: Shell, version: &str) -> (PathBuf, String) {
        let archive_root = root.join(format!("{}-{version}", shell.as_str()));
        let bin_dir = archive_root.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let shell_path = bin_dir.join(shell.as_str());
        let script = match shell {
            Shell::Bash | Shell::Zsh => format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf '{} {}\\n'\n  exit 0\nfi\nprintf '%s\\n' \"${{SHUCK_SHELL_VERSION}}\"\n",
                shell.as_str(),
                version
            ),
            Shell::Dash => format!(
                "#!/bin/sh\nif [ \"$1\" = \"-V\" ] || [ \"$1\" = \"--version\" ]; then\n  printf '{} {}\\n' 1>&2\n  exit 0\nfi\nprintf '%s\\n' \"${{SHUCK_SHELL_VERSION}}\"\n",
                shell.as_str(),
                version
            ),
            Shell::Mksh => format!(
                "#!/bin/sh\nif [ \"$1\" = \"-c\" ]; then\n  printf '@(#)MIRBSD KSH R{}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"-V\" ]; then\n  printf '@(#)MIRBSD KSH R{}\\n'\n  exit 0\nfi\nprintf '%s\\n' \"${{SHUCK_SHELL_VERSION}}\"\n",
                version, version
            ),
        };
        fs::write(&shell_path, script).unwrap();
        let mut permissions = fs::metadata(&shell_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&shell_path, permissions).unwrap();

        let archive_path = root.join(format!("{}-{version}.tar.gz", shell.as_str()));
        let status = Command::new("/usr/bin/tar")
            .current_dir(&archive_root)
            .arg("-czf")
            .arg(&archive_path)
            .arg("bin")
            .status()
            .unwrap();
        assert!(status.success());

        let digest = Sha256::digest(fs::read(&archive_path).unwrap());
        let sha256 = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        (archive_path, sha256)
    }

    fn registry_for_archive(shell: Shell, version: &str, archive: &Path, sha256: &str) -> String {
        let platform = current_platform().unwrap();
        format!(
            r#"{{
  "version": 1,
  "shells": {{
    "{shell}": {{
      "versions": {{
        "{version}": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url}",
              "sha256": "{sha256}"
            }}
          }}
        }}
      }}
    }}
  }}
}}"#,
            shell = shell.as_str(),
            version = version,
            platform = platform,
            url = Url::from_file_path(archive).unwrap(),
            sha256 = sha256
        )
    }

    #[test]
    fn parses_exact_and_range_constraints() {
        assert!(matches!(
            VersionConstraint::parse("latest").unwrap(),
            VersionConstraint::Latest
        ));
        assert!(matches!(
            VersionConstraint::parse("5.2").unwrap(),
            VersionConstraint::ExactPrefix(_)
        ));
        assert!(matches!(
            VersionConstraint::parse("5.2.21").unwrap(),
            VersionConstraint::Exact(_)
        ));
        assert!(matches!(
            VersionConstraint::parse(">=5.1,<6").unwrap(),
            VersionConstraint::Range(_)
        ));
    }

    #[test]
    fn resolves_cli_shell_and_config_version() {
        let tempdir = tempfile::tempdir().unwrap();
        let (archive, sha256) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
        let registry = registry_for_archive(Shell::Bash, "5.2.21", &archive, &sha256);
        let registry_path = write_registry(tempdir.path(), &registry);
        let environment = test_environment(
            tempdir.path(),
            Url::from_file_path(registry_path).unwrap().to_string(),
        );

        let config = RunConfig {
            shell: None,
            shell_version: None,
            shells: BTreeMap::from([(String::from("bash"), String::from("5.2"))]),
        };
        let resolved = resolve_with_environment(
            &environment,
            ResolveOptions {
                shell: Some(Shell::Bash),
                version: None,
                system: false,
                script: None,
                config: Some(&config),
                verbose: false,
                refresh_registry: false,
            },
        )
        .unwrap();

        assert_eq!(resolved.shell, Shell::Bash);
        assert_eq!(resolved.version.as_str(), "5.2.21");
        assert_eq!(resolved.source, ResolutionSource::Managed);
        assert!(resolved.path.ends_with("bin/bash"));
    }

    #[test]
    fn metadata_overrides_project_defaults() {
        let tempdir = tempfile::tempdir().unwrap();
        let (archive, sha256) = make_shell_archive(tempdir.path(), Shell::Zsh, "5.9");
        let registry = registry_for_archive(Shell::Zsh, "5.9", &archive, &sha256);
        let registry_path = write_registry(tempdir.path(), &registry);
        let environment = test_environment(
            tempdir.path(),
            Url::from_file_path(registry_path).unwrap().to_string(),
        );
        let script_path = tempdir.path().join("deploy.sh");
        fs::write(
            &script_path,
            "# /// shuck\n# shell = \"zsh\"\n# version = \"5.9\"\n# ///\nprint hello\n",
        )
        .unwrap();
        let config = RunConfig {
            shell: Some(String::from("bash")),
            shell_version: Some(String::from("5.2")),
            shells: BTreeMap::new(),
        };

        let resolved = resolve_with_environment(
            &environment,
            ResolveOptions {
                shell: None,
                version: None,
                system: false,
                script: Some(&script_path),
                config: Some(&config),
                verbose: false,
                refresh_registry: false,
            },
        )
        .unwrap();

        assert_eq!(resolved.shell, Shell::Zsh);
        assert_eq!(resolved.version.as_str(), "5.9");
    }

    #[test]
    fn shell_specific_config_pin_overrides_generic_shell_version() {
        let tempdir = tempfile::tempdir().unwrap();
        let (archive_a, sha_a) = make_shell_archive(tempdir.path(), Shell::Bash, "5.1.16");
        let (archive_b, sha_b) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
        let platform = current_platform().unwrap();
        let registry = format!(
            r#"{{
  "version": 1,
  "shells": {{
    "bash": {{
      "versions": {{
        "5.1.16": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url_a}",
              "sha256": "{sha_a}"
            }}
          }}
        }},
        "5.2.21": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url_b}",
              "sha256": "{sha_b}"
            }}
          }}
        }}
      }}
    }}
  }}
}}"#,
            platform = platform,
            url_a = Url::from_file_path(archive_a).unwrap(),
            sha_a = sha_a,
            url_b = Url::from_file_path(archive_b).unwrap(),
            sha_b = sha_b
        );
        let registry_path = write_registry(tempdir.path(), &registry);
        let environment = test_environment(
            tempdir.path(),
            Url::from_file_path(registry_path).unwrap().to_string(),
        );
        let script_path = tempdir.path().join("deploy.sh");
        fs::write(&script_path, "#!/usr/bin/env bash\necho hi\n").unwrap();
        let config = RunConfig {
            shell: None,
            shell_version: Some(String::from("5.1")),
            shells: BTreeMap::from([(String::from("bash"), String::from("5.2"))]),
        };

        let resolved = resolve_with_environment(
            &environment,
            ResolveOptions {
                shell: None,
                version: None,
                system: false,
                script: Some(&script_path),
                config: Some(&config),
                verbose: false,
                refresh_registry: false,
            },
        )
        .unwrap();

        assert_eq!(resolved.shell, Shell::Bash);
        assert_eq!(resolved.version.as_str(), "5.2.21");
    }

    #[test]
    fn shebang_without_other_constraints_uses_latest_available_version() {
        let tempdir = tempfile::tempdir().unwrap();
        let (archive_a, sha_a) = make_shell_archive(tempdir.path(), Shell::Bash, "5.1.16");
        let (archive_b, sha_b) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
        let platform = current_platform().unwrap();
        let registry = format!(
            r#"{{
  "version": 1,
  "shells": {{
    "bash": {{
      "versions": {{
        "5.1.16": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url_a}",
              "sha256": "{sha_a}"
            }}
          }}
        }},
        "5.2.21": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url_b}",
              "sha256": "{sha_b}"
            }}
          }}
        }}
      }}
    }}
  }}
}}"#,
            platform = platform,
            url_a = Url::from_file_path(archive_a).unwrap(),
            sha_a = sha_a,
            url_b = Url::from_file_path(archive_b).unwrap(),
            sha_b = sha_b
        );
        let registry_path = write_registry(tempdir.path(), &registry);
        let environment = test_environment(
            tempdir.path(),
            Url::from_file_path(registry_path).unwrap().to_string(),
        );
        let script_path = tempdir.path().join("deploy.sh");
        fs::write(&script_path, "#!/usr/bin/env bash\necho hi\n").unwrap();

        let resolved = resolve_with_environment(
            &environment,
            ResolveOptions {
                shell: None,
                version: None,
                system: false,
                script: Some(&script_path),
                config: None,
                verbose: false,
                refresh_registry: false,
            },
        )
        .unwrap();

        assert_eq!(resolved.shell, Shell::Bash);
        assert_eq!(resolved.version.as_str(), "5.2.21");
    }

    #[test]
    fn checksum_mismatch_aborts_install() {
        let tempdir = tempfile::tempdir().unwrap();
        let (archive, _sha256) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
        let registry = registry_for_archive(
            Shell::Bash,
            "5.2.21",
            &archive,
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        let registry_path = write_registry(tempdir.path(), &registry);
        let environment = test_environment(
            tempdir.path(),
            Url::from_file_path(registry_path).unwrap().to_string(),
        );

        let err = install_with_environment(
            &environment,
            Shell::Bash,
            &VersionConstraint::parse("5.2").unwrap(),
            false,
            false,
        )
        .unwrap_err();

        assert!(format!("{err:#}").contains("Checksum mismatch"));
    }

    #[test]
    fn parses_script_metadata_before_non_comment_lines() {
        let metadata = parse_script_metadata(
            "# /// shuck\n# shell = \"bash\"\n# version = \">=5.1\"\n# [metadata]\n# description = \"demo\"\n# ///\necho hi\n",
        )
        .unwrap()
        .unwrap();
        assert_eq!(metadata.shell, Shell::Bash);
        assert!(matches!(
            metadata.version.unwrap(),
            VersionConstraint::Range(_)
        ));

        let err =
            parse_script_metadata("echo hi\n# /// shuck\n# shell = \"bash\"\n# ///\n").unwrap_err();
        assert!(err.to_string().contains("before the script body"));
    }

    #[test]
    fn rejects_unknown_metadata_keys() {
        assert!(
            parse_script_metadata("# /// shuck\n# shell = \"bash\"\n# foo = \"bar\"\n# ///\n")
                .is_err()
        );
    }

    #[test]
    fn lists_available_versions() {
        let tempdir = tempfile::tempdir().unwrap();
        let (archive_a, sha_a) = make_shell_archive(tempdir.path(), Shell::Bash, "5.1.16");
        let (archive_b, sha_b) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
        let platform = current_platform().unwrap();
        let registry = format!(
            r#"{{
  "version": 1,
  "shells": {{
    "bash": {{
      "versions": {{
        "5.1.16": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url_a}",
              "sha256": "{sha_a}"
            }}
          }}
        }},
        "5.2.21": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url_b}",
              "sha256": "{sha_b}"
            }}
          }}
        }}
      }}
    }}
  }}
}}"#,
            platform = platform,
            url_a = Url::from_file_path(archive_a).unwrap(),
            sha_a = sha_a,
            url_b = Url::from_file_path(archive_b).unwrap(),
            sha_b = sha_b
        );
        let registry_path = write_registry(tempdir.path(), &registry);
        let environment = test_environment(
            tempdir.path(),
            Url::from_file_path(registry_path).unwrap().to_string(),
        );

        let available = available_shells(
            &load_registry(&environment, false, false).unwrap(),
            Some(Shell::Bash),
        );
        assert_eq!(available.len(), 1);
        assert_eq!(available[0].versions[0].as_str(), "5.2.21");
        assert_eq!(available[0].versions[1].as_str(), "5.1.16");
    }

    #[test]
    fn system_resolution_checks_version_constraints() {
        let tempdir = tempfile::tempdir().unwrap();
        let path_dir = tempdir.path().join("bin");
        fs::create_dir_all(&path_dir).unwrap();
        let shell_path = path_dir.join("bash");
        fs::write(
            &shell_path,
            "#!/bin/sh\nprintf 'GNU bash, version 5.2.21(1)-release\\n'\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&shell_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&shell_path, permissions).unwrap();

        let resolved = resolve_system_at_path(
            Shell::Bash,
            &shell_path,
            &VersionConstraint::parse(">=5.1,<6").unwrap(),
        )
        .unwrap();
        assert_eq!(resolved.version.as_str(), "5.2.21");
        let err = resolve_system_at_path(
            Shell::Bash,
            &shell_path,
            &VersionConstraint::parse(">=6").unwrap(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("System bash is 5.2.21"));
    }
}
