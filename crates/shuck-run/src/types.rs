use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;

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
    pub implicit_system_fallback: bool,
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
            implicit_system_fallback: false,
            script,
            config,
            verbose: false,
            refresh_registry: false,
        }
    }
}

pub(crate) fn parse_shell_name(raw: &str) -> Result<Shell> {
    Shell::from_name(raw)
        .ok_or_else(|| anyhow!("unsupported shell `{raw}`; expected one of: bash, zsh, dash, mksh"))
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
