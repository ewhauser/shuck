use std::path::Path;

/// Indentation style used by the shell formatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndentStyle {
    /// Use tab characters for indentation.
    #[default]
    Tab,
    /// Use spaces for indentation.
    Space,
}

/// Line ending style detected or emitted by formatter operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// Unix line endings.
    #[default]
    Lf,
    /// Windows line endings.
    CrLf,
}

/// Shell dialect requested for formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellDialect {
    /// Infer the dialect from the shebang or path.
    #[default]
    Auto,
    /// Bash syntax.
    Bash,
    /// POSIX `sh` syntax.
    Posix,
    /// MirBSD Korn shell syntax.
    Mksh,
    /// Z shell syntax.
    Zsh,
}

impl ShellDialect {
    /// Resolve [`Self::Auto`] using source text and an optional path.
    #[must_use]
    pub fn resolve(self, source: &str, path: Option<&Path>) -> Self {
        match self {
            Self::Auto => infer_dialect(source, path),
            dialect => dialect,
        }
    }
}

/// User-configurable shell formatter options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellFormatOptions {
    dialect: ShellDialect,
    indent_style: IndentStyle,
    indent_width: u8,
    binary_next_line: bool,
    switch_case_indent: bool,
    space_redirects: bool,
    keep_padding: bool,
    function_next_line: bool,
    never_split: bool,
    simplify: bool,
    minify: bool,
}

impl Default for ShellFormatOptions {
    fn default() -> Self {
        Self {
            dialect: ShellDialect::Auto,
            indent_style: IndentStyle::Tab,
            indent_width: 8,
            binary_next_line: false,
            switch_case_indent: false,
            space_redirects: false,
            keep_padding: false,
            function_next_line: false,
            never_split: false,
            simplify: false,
            minify: false,
        }
    }
}

impl ShellFormatOptions {
    /// Return the requested shell dialect.
    #[must_use]
    pub fn dialect(&self) -> ShellDialect {
        self.dialect
    }

    /// Return the requested indentation style.
    #[must_use]
    pub fn indent_style(&self) -> IndentStyle {
        self.indent_style
    }

    /// Return the requested indentation width.
    #[must_use]
    pub fn indent_width(&self) -> u8 {
        self.indent_width
    }

    /// Return whether binary operators should begin continuation lines.
    #[must_use]
    pub fn binary_next_line(&self) -> bool {
        self.binary_next_line
    }

    /// Return whether `case` arms should receive an extra indentation level.
    #[must_use]
    pub fn switch_case_indent(&self) -> bool {
        self.switch_case_indent
    }

    /// Return whether redirection operators should be surrounded by spaces.
    #[must_use]
    pub fn space_redirects(&self) -> bool {
        self.space_redirects
    }

    /// Return whether existing horizontal padding should be preserved.
    #[must_use]
    pub fn keep_padding(&self) -> bool {
        self.keep_padding
    }

    /// Return whether function bodies should start on the next line.
    #[must_use]
    pub fn function_next_line(&self) -> bool {
        self.function_next_line
    }

    /// Return whether the formatter should avoid splitting lines.
    #[must_use]
    pub fn never_split(&self) -> bool {
        self.never_split
    }

    /// Return whether simplification passes are enabled.
    #[must_use]
    pub fn simplify(&self) -> bool {
        self.simplify
    }

    /// Return whether compact minified output is requested.
    #[must_use]
    pub fn minify(&self) -> bool {
        self.minify
    }

    /// Return a copy with a different shell dialect setting.
    #[must_use]
    pub fn with_dialect(mut self, dialect: ShellDialect) -> Self {
        self.dialect = dialect;
        self
    }

    /// Return a copy with a different indentation style.
    #[must_use]
    pub fn with_indent_style(mut self, indent_style: IndentStyle) -> Self {
        self.indent_style = indent_style;
        self
    }

    /// Return a copy with a different indentation width.
    ///
    /// Values less than one are clamped to one.
    #[must_use]
    pub fn with_indent_width(mut self, indent_width: u8) -> Self {
        self.indent_width = indent_width.max(1);
        self
    }

    /// Return a copy with binary-next-line formatting enabled or disabled.
    #[must_use]
    pub fn with_binary_next_line(mut self, enabled: bool) -> Self {
        self.binary_next_line = enabled;
        self
    }

    /// Return a copy with switch-case indentation enabled or disabled.
    #[must_use]
    pub fn with_switch_case_indent(mut self, enabled: bool) -> Self {
        self.switch_case_indent = enabled;
        self
    }

    /// Return a copy with spaced redirections enabled or disabled.
    #[must_use]
    pub fn with_space_redirects(mut self, enabled: bool) -> Self {
        self.space_redirects = enabled;
        self
    }

    /// Return a copy with padding preservation enabled or disabled.
    #[must_use]
    pub fn with_keep_padding(mut self, enabled: bool) -> Self {
        self.keep_padding = enabled;
        self
    }

    /// Return a copy with function-next-line formatting enabled or disabled.
    #[must_use]
    pub fn with_function_next_line(mut self, enabled: bool) -> Self {
        self.function_next_line = enabled;
        self
    }

    /// Return a copy with line splitting enabled or disabled.
    #[must_use]
    pub fn with_never_split(mut self, enabled: bool) -> Self {
        self.never_split = enabled;
        self
    }

    /// Return a copy with simplification enabled or disabled.
    #[must_use]
    pub fn with_simplify(mut self, enabled: bool) -> Self {
        self.simplify = enabled;
        self
    }

    /// Return a copy with minified output enabled or disabled.
    #[must_use]
    pub fn with_minify(mut self, enabled: bool) -> Self {
        self.minify = enabled;
        self
    }

    /// Resolve inferred options against concrete source text and path metadata.
    #[must_use]
    pub fn resolve(&self, source: &str, path: Option<&Path>) -> ResolvedShellFormatOptions {
        ResolvedShellFormatOptions {
            dialect: self.dialect.resolve(source, path),
            indent_style: self.indent_style,
            indent_width: self.indent_width.max(1),
            binary_next_line: self.binary_next_line,
            switch_case_indent: self.switch_case_indent,
            space_redirects: self.space_redirects,
            keep_padding: self.keep_padding,
            function_next_line: self.function_next_line,
            never_split: self.never_split,
            simplify: self.simplify,
            minify: self.minify,
            line_ending: detect_line_ending(source),
        }
    }
}

/// Formatter options after source-dependent defaults have been resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedShellFormatOptions {
    dialect: ShellDialect,
    indent_style: IndentStyle,
    indent_width: u8,
    binary_next_line: bool,
    switch_case_indent: bool,
    space_redirects: bool,
    keep_padding: bool,
    function_next_line: bool,
    never_split: bool,
    simplify: bool,
    minify: bool,
    line_ending: LineEnding,
}

impl ResolvedShellFormatOptions {
    /// Return the resolved shell dialect.
    #[must_use]
    pub fn dialect(&self) -> ShellDialect {
        self.dialect
    }

    /// Return the resolved indentation style.
    #[must_use]
    pub fn indent_style(&self) -> IndentStyle {
        self.indent_style
    }

    /// Return the resolved indentation width.
    #[must_use]
    pub fn indent_width(&self) -> u8 {
        self.indent_width
    }

    /// Return whether binary operators should begin continuation lines.
    #[must_use]
    pub fn binary_next_line(&self) -> bool {
        self.binary_next_line
    }

    /// Return whether `case` arms should receive an extra indentation level.
    #[must_use]
    pub fn switch_case_indent(&self) -> bool {
        self.switch_case_indent
    }

    /// Return whether redirection operators should be surrounded by spaces.
    #[must_use]
    pub fn space_redirects(&self) -> bool {
        self.space_redirects
    }

    /// Return whether existing horizontal padding should be preserved.
    #[must_use]
    pub fn keep_padding(&self) -> bool {
        self.keep_padding
    }

    /// Return whether function bodies should start on the next line.
    #[must_use]
    pub fn function_next_line(&self) -> bool {
        self.function_next_line
    }

    /// Return whether the formatter should avoid splitting lines.
    #[must_use]
    pub fn never_split(&self) -> bool {
        self.never_split
    }

    /// Return whether simplification passes are enabled.
    #[must_use]
    pub fn simplify(&self) -> bool {
        self.simplify
    }

    /// Return whether compact minified output is requested.
    #[must_use]
    pub fn minify(&self) -> bool {
        self.minify
    }

    /// Return whether the resolved configuration prefers compact layout.
    #[must_use]
    pub fn compact_layout(&self) -> bool {
        self.minify || self.never_split
    }

    /// Return the detected line ending style.
    #[must_use]
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }
}

fn infer_dialect(source: &str, path: Option<&Path>) -> ShellDialect {
    if let Some(first_line) = source.lines().next()
        && let Some(interpreter) = interpreter_name(first_line)
    {
        return ShellDialect::from_name(interpreter);
    }

    path.and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .map(ShellDialect::from_name)
        .unwrap_or(ShellDialect::Bash)
}

impl ShellDialect {
    fn from_name(name: &str) -> Self {
        match name {
            "sh" | "dash" | "ksh" | "posix" => Self::Posix,
            "mksh" => Self::Mksh,
            "zsh" => Self::Zsh,
            _ => Self::Bash,
        }
    }
}

fn interpreter_name(line: &str) -> Option<&str> {
    let shebang = line.strip_prefix("#!")?.trim_start();
    let command = shebang
        .split_whitespace()
        .find(|part| !part.starts_with('-'))?;
    command.rsplit('/').next()
}

fn detect_line_ending(source: &str) -> LineEnding {
    if source.contains("\r\n") {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    }
}
