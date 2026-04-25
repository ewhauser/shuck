use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndentStyle {
    #[default]
    Tab,
    Space,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellDialect {
    #[default]
    Auto,
    Bash,
    Posix,
    Mksh,
    Zsh,
}

impl ShellDialect {
    #[must_use]
    pub fn resolve(self, source: &str, path: Option<&Path>) -> Self {
        match self {
            Self::Auto => infer_dialect(source, path),
            dialect => dialect,
        }
    }
}

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
    #[must_use]
    pub fn dialect(&self) -> ShellDialect {
        self.dialect
    }

    #[must_use]
    pub fn indent_style(&self) -> IndentStyle {
        self.indent_style
    }

    #[must_use]
    pub fn indent_width(&self) -> u8 {
        self.indent_width
    }

    #[must_use]
    pub fn binary_next_line(&self) -> bool {
        self.binary_next_line
    }

    #[must_use]
    pub fn switch_case_indent(&self) -> bool {
        self.switch_case_indent
    }

    #[must_use]
    pub fn space_redirects(&self) -> bool {
        self.space_redirects
    }

    #[must_use]
    pub fn keep_padding(&self) -> bool {
        self.keep_padding
    }

    #[must_use]
    pub fn function_next_line(&self) -> bool {
        self.function_next_line
    }

    #[must_use]
    pub fn never_split(&self) -> bool {
        self.never_split
    }

    #[must_use]
    pub fn simplify(&self) -> bool {
        self.simplify
    }

    #[must_use]
    pub fn minify(&self) -> bool {
        self.minify
    }

    #[must_use]
    pub fn with_dialect(mut self, dialect: ShellDialect) -> Self {
        self.dialect = dialect;
        self
    }

    #[must_use]
    pub fn with_indent_style(mut self, indent_style: IndentStyle) -> Self {
        self.indent_style = indent_style;
        self
    }

    #[must_use]
    pub fn with_indent_width(mut self, indent_width: u8) -> Self {
        self.indent_width = indent_width.max(1);
        self
    }

    #[must_use]
    pub fn with_binary_next_line(mut self, enabled: bool) -> Self {
        self.binary_next_line = enabled;
        self
    }

    #[must_use]
    pub fn with_switch_case_indent(mut self, enabled: bool) -> Self {
        self.switch_case_indent = enabled;
        self
    }

    #[must_use]
    pub fn with_space_redirects(mut self, enabled: bool) -> Self {
        self.space_redirects = enabled;
        self
    }

    #[must_use]
    pub fn with_keep_padding(mut self, enabled: bool) -> Self {
        self.keep_padding = enabled;
        self
    }

    #[must_use]
    pub fn with_function_next_line(mut self, enabled: bool) -> Self {
        self.function_next_line = enabled;
        self
    }

    #[must_use]
    pub fn with_never_split(mut self, enabled: bool) -> Self {
        self.never_split = enabled;
        self
    }

    #[must_use]
    pub fn with_simplify(mut self, enabled: bool) -> Self {
        self.simplify = enabled;
        self
    }

    #[must_use]
    pub fn with_minify(mut self, enabled: bool) -> Self {
        self.minify = enabled;
        self
    }

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
    #[must_use]
    pub fn dialect(&self) -> ShellDialect {
        self.dialect
    }

    #[must_use]
    pub fn indent_style(&self) -> IndentStyle {
        self.indent_style
    }

    #[must_use]
    pub fn indent_width(&self) -> u8 {
        self.indent_width
    }

    #[must_use]
    pub fn binary_next_line(&self) -> bool {
        self.binary_next_line
    }

    #[must_use]
    pub fn switch_case_indent(&self) -> bool {
        self.switch_case_indent
    }

    #[must_use]
    pub fn space_redirects(&self) -> bool {
        self.space_redirects
    }

    #[must_use]
    pub fn keep_padding(&self) -> bool {
        self.keep_padding
    }

    #[must_use]
    pub fn function_next_line(&self) -> bool {
        self.function_next_line
    }

    #[must_use]
    pub fn never_split(&self) -> bool {
        self.never_split
    }

    #[must_use]
    pub fn simplify(&self) -> bool {
        self.simplify
    }

    #[must_use]
    pub fn minify(&self) -> bool {
        self.minify
    }

    #[must_use]
    pub fn compact_layout(&self) -> bool {
        self.minify || self.never_split
    }

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
