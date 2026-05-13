use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;

/// Supported shell families for `shuck run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Shell {
    /// GNU Bash.
    Bash,
    /// Homebrew-style GNU Bash alias.
    Gbash,
    /// Bashkit-managed Bash.
    Bashkit,
    /// Z shell.
    Zsh,
    /// Debian Almquist shell or POSIX `sh`.
    Dash,
    /// MirBSD Korn shell.
    Mksh,
    /// BusyBox shell applet.
    Busybox,
}

impl Shell {
    /// Return the canonical config and CLI spelling for this shell.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Gbash => "gbash",
            Self::Bashkit => "bashkit",
            Self::Zsh => "zsh",
            Self::Dash => "dash",
            Self::Mksh => "mksh",
            Self::Busybox => "busybox",
        }
    }

    /// Parse a shell name or common alias.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "bash" => Some(Self::Bash),
            "gbash" => Some(Self::Gbash),
            "bashkit" => Some(Self::Bashkit),
            "zsh" => Some(Self::Zsh),
            "dash" | "sh" => Some(Self::Dash),
            "mksh" | "ksh" => Some(Self::Mksh),
            "busybox" => Some(Self::Busybox),
            _ => None,
        }
    }

    pub(crate) fn ensure_supported_on_current_platform(self) -> Result<()> {
        if self != Self::Busybox || std::env::consts::OS == "linux" {
            return Ok(());
        }

        bail!("busybox is only supported on Linux");
    }

    pub(crate) fn infer(source: &str, path: Option<&Path>) -> Option<Self> {
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

/// Parsed shell version with ordering semantics for registry selection.
#[derive(Debug, Clone)]
pub struct Version {
    raw: String,
    tokens: Vec<VersionToken>,
    segment_count: usize,
    prefix_match: bool,
}

impl Version {
    /// Parse a version string accepted by Shuck's shell registry.
    pub fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            bail!("version cannot be empty");
        }
        if !raw
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.')
        {
            bail!("invalid version `{raw}`");
        }
        if raw.split('.').any(|segment| segment.is_empty()) {
            bail!("invalid version `{raw}`");
        }

        let tokens = tokenize_version(raw)?;
        if tokens.is_empty() {
            bail!("invalid version `{raw}`");
        }

        let segment_count = raw.split('.').count();
        let prefix_match = should_treat_as_prefix(raw, &tokens, segment_count);

        Ok(Self {
            raw: raw.to_owned(),
            tokens,
            segment_count,
            prefix_match,
        })
    }

    /// Return the original version string.
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

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

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

fn tokenize_version(raw: &str) -> Result<Vec<VersionToken>> {
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
            let value = digits
                .parse::<u64>()
                .map_err(|_| anyhow!("invalid version `{raw}`"))?;
            tokens.push(VersionToken::Numeric(value));
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

    Ok(tokens)
}

fn should_treat_as_prefix(raw: &str, tokens: &[VersionToken], segment_count: usize) -> bool {
    raw.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
        && segment_count == 2
        && tokens
            .iter()
            .all(|token| matches!(token, VersionToken::Numeric(_)))
}

/// Version requirement used when resolving or installing a shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionConstraint {
    /// Select the newest available version.
    Latest,
    /// Select exactly this version.
    Exact(Version),
    /// Select versions matching this dotted numeric prefix.
    ExactPrefix(Version),
    /// Select versions satisfying every predicate.
    Range(Vec<VersionPredicate>),
}

impl VersionConstraint {
    /// Parse `latest`, an exact version, or a comma-separated predicate range.
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

    /// Return whether `version` satisfies this constraint.
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

    pub(crate) fn describe(&self) -> String {
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

/// Managed versions available for one shell family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableShell {
    /// Shell family.
    pub shell: Shell,
    /// Available versions for that shell.
    pub versions: Vec<Version>,
}

/// Runtime shell settings loaded from Shuck config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct RunConfig {
    /// Default shell name.
    pub shell: Option<String>,
    /// Default shell version constraint.
    pub shell_version: Option<String>,
    /// Per-shell version constraints keyed by shell name.
    pub shells: BTreeMap<String, String>,
}

/// Concrete interpreter selected for a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInterpreter {
    /// Shell family that was resolved.
    pub shell: Shell,
    /// Shell version selected for execution.
    pub version: Version,
    /// Filesystem path to the executable interpreter.
    pub path: PathBuf,
    /// Where the interpreter was found.
    pub source: ResolutionSource,
}

/// Source used to resolve an interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionSource {
    /// Interpreter came from Shuck's managed shell cache.
    Managed,
    /// Interpreter came from the host system.
    System,
}

/// Options controlling interpreter resolution.
#[derive(Debug, Clone)]
pub struct ResolveOptions<'a> {
    /// Requested shell family.
    pub shell: Option<Shell>,
    /// Requested shell version constraint.
    pub version: Option<VersionConstraint>,
    /// Whether to require a system interpreter.
    pub system: bool,
    /// Whether managed resolution may fall back to the system interpreter.
    pub implicit_system_fallback: bool,
    /// Optional script path used for shell inference.
    pub script: Option<&'a Path>,
    /// Optional config values used as defaults.
    pub config: Option<&'a RunConfig>,
    /// Whether resolution should print progress details.
    pub verbose: bool,
    /// Whether to refresh the registry before resolving.
    pub refresh_registry: bool,
}

impl<'a> ResolveOptions<'a> {
    /// Create resolution options from the common CLI inputs.
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
            implicit_system_fallback: false,
            script,
            config,
            verbose: false,
            refresh_registry: false,
        }
    }
}

pub(crate) fn parse_shell_name(raw: &str) -> Result<Shell> {
    Shell::from_name(raw).ok_or_else(|| {
        anyhow!(
            "unsupported shell `{raw}`; expected one of: bash, gbash, bashkit, zsh, dash, mksh, busybox"
        )
    })
}

/// A single comparison predicate inside a version range.
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
