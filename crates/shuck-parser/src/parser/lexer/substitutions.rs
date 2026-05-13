use super::*;

impl<'a> Lexer<'a> {
    pub(in crate::parser) fn read_arithmetic_expansion_into(
        &mut self,
        content: &mut Option<String>,
    ) -> bool {
        debug_assert_eq!(self.peek_char(), Some('('));
        debug_assert_eq!(self.second_char(), Some('('));

        Self::push_capture_char(content, '(');
        self.advance();
        Self::push_capture_char(content, '(');
        self.advance();

        let mut depth = 2;
        while let Some(c) = self.peek_char() {
            match c {
                '\\' => {
                    Self::push_capture_char(content, c);
                    self.advance();
                    if let Some(next) = self.peek_char() {
                        Self::push_capture_char(content, next);
                        self.advance();
                    }
                }
                '\'' => {
                    Self::push_capture_char(content, c);
                    self.advance();
                    while let Some(quoted) = self.peek_char() {
                        Self::push_capture_char(content, quoted);
                        self.advance();
                        if quoted == '\'' {
                            break;
                        }
                    }
                }
                '"' => {
                    let mut escaped = false;
                    Self::push_capture_char(content, c);
                    self.advance();
                    while let Some(quoted) = self.peek_char() {
                        Self::push_capture_char(content, quoted);
                        self.advance();
                        if escaped {
                            escaped = false;
                            continue;
                        }
                        match quoted {
                            '\\' => escaped = true,
                            '"' => break,
                            _ => {}
                        }
                    }
                }
                '`' => {
                    let mut escaped = false;
                    Self::push_capture_char(content, c);
                    self.advance();
                    while let Some(quoted) = self.peek_char() {
                        Self::push_capture_char(content, quoted);
                        self.advance();
                        if escaped {
                            escaped = false;
                            continue;
                        }
                        match quoted {
                            '\\' => escaped = true,
                            '`' => break,
                            _ => {}
                        }
                    }
                }
                '(' => {
                    Self::push_capture_char(content, c);
                    self.advance();
                    depth += 1;
                }
                ')' => {
                    Self::push_capture_char(content, c);
                    self.advance();
                    depth -= 1;
                    if depth == 0 {
                        return true;
                    }
                }
                _ => {
                    Self::push_capture_char(content, c);
                    self.advance();
                }
            }
        }

        false
    }

    pub(in crate::parser) fn read_legacy_arithmetic_into(
        &mut self,
        content: &mut Option<String>,
        segment_start: Position,
    ) -> bool {
        let mut bracket_depth = 1;

        while let Some(c) = self.peek_char() {
            match c {
                '\\' => {
                    Self::push_capture_char(content, c);
                    self.advance();
                    if let Some(next) = self.peek_char() {
                        Self::push_capture_char(content, next);
                        self.advance();
                    }
                }
                '\'' => {
                    Self::push_capture_char(content, c);
                    self.advance();
                    while let Some(quoted) = self.peek_char() {
                        Self::push_capture_char(content, quoted);
                        self.advance();
                        if quoted == '\'' {
                            break;
                        }
                    }
                }
                '"' => {
                    let mut escaped = false;
                    Self::push_capture_char(content, c);
                    self.advance();
                    while let Some(quoted) = self.peek_char() {
                        Self::push_capture_char(content, quoted);
                        self.advance();
                        if escaped {
                            escaped = false;
                            continue;
                        }
                        match quoted {
                            '\\' => escaped = true,
                            '"' => break,
                            _ => {}
                        }
                    }
                }
                '`' => {
                    let mut escaped = false;
                    Self::push_capture_char(content, c);
                    self.advance();
                    while let Some(quoted) = self.peek_char() {
                        Self::push_capture_char(content, quoted);
                        self.advance();
                        if escaped {
                            escaped = false;
                            continue;
                        }
                        match quoted {
                            '\\' => escaped = true,
                            '`' => break,
                            _ => {}
                        }
                    }
                }
                '[' => {
                    Self::push_capture_char(content, c);
                    self.advance();
                    bracket_depth += 1;
                }
                ']' => {
                    Self::push_capture_char(content, c);
                    self.advance();
                    bracket_depth -= 1;
                    if bracket_depth == 0 {
                        return true;
                    }
                }
                '$' => {
                    Self::push_capture_char(content, c);
                    self.advance();
                    if self.peek_char() == Some('(') {
                        if self.second_char() == Some('(') {
                            if !self.read_arithmetic_expansion_into(content) {
                                return false;
                            }
                        } else {
                            Self::push_capture_char(content, '(');
                            self.advance();
                            if !self.read_command_subst_into(content) {
                                return false;
                            }
                        }
                    } else if self.peek_char() == Some('{') {
                        Self::push_capture_char(content, '{');
                        self.advance();
                        if !self.read_param_expansion_into(content, segment_start) {
                            return false;
                        }
                    } else if self.peek_char() == Some('[') {
                        Self::push_capture_char(content, '[');
                        self.advance();
                        if !self.read_legacy_arithmetic_into(content, segment_start) {
                            return false;
                        }
                    }
                }
                _ => {
                    Self::push_capture_char(content, c);
                    self.advance();
                }
            }
        }

        false
    }

    /// Read command substitution content after `$(`, handling nested parens and quotes.
    /// Appends chars to `content` and adds the closing `)`.
    /// `subst_depth` tracks nesting to prevent stack overflow.
    pub(in crate::parser) fn read_command_subst_into(
        &mut self,
        content: &mut Option<String>,
    ) -> bool {
        self.read_command_subst_into_depth(content, 0)
    }

    pub(in crate::parser) fn flush_command_subst_keyword(
        current_word: &mut String,
        pending_case_headers: &mut usize,
        case_clause_depths: &mut SmallVec<[usize; 4]>,
        depth: usize,
        word_started_at_command_start: &mut bool,
    ) {
        if current_word.is_empty() {
            *word_started_at_command_start = false;
            return;
        }

        match current_word.as_str() {
            "case" if *word_started_at_command_start => *pending_case_headers += 1,
            "in" if *pending_case_headers > 0 => {
                *pending_case_headers -= 1;
                case_clause_depths.push(depth);
            }
            "esac" if *word_started_at_command_start => {
                case_clause_depths.pop();
            }
            _ => {}
        }

        current_word.clear();
        *word_started_at_command_start = false;
    }

    pub(in crate::parser) fn read_command_subst_heredoc_delimiter_into(
        &mut self,
        content: &mut Option<String>,
    ) -> Option<String> {
        while let Some(ch) = self.peek_char() {
            if !matches!(ch, ' ' | '\t') {
                break;
            }
            Self::push_capture_char(content, ch);
            self.advance();
        }

        let mut cooked = String::new();
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;
        let mut saw_any = false;

        while let Some(ch) = self.peek_char() {
            if heredoc_delimiter_is_terminator(ch, in_single, in_double, escaped) {
                break;
            }

            saw_any = true;
            Self::push_capture_char(content, ch);
            self.advance();

            if escaped {
                cooked.push(ch);
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                _ => cooked.push(ch),
            }
        }

        saw_any.then_some(cooked)
    }

    pub(in crate::parser) fn read_command_subst_backtick_segment_into(
        &mut self,
        content: &mut Option<String>,
    ) {
        Self::push_capture_char(content, '`');
        self.advance();
        while let Some(ch) = self.peek_char() {
            Self::push_capture_char(content, ch);
            self.advance();
            if ch == '\\' {
                if let Some(esc) = self.peek_char() {
                    Self::push_capture_char(content, esc);
                    self.advance();
                }
                continue;
            }
            if ch == '`' {
                break;
            }
        }
    }

    pub(in crate::parser) fn read_command_subst_pending_heredoc_into(
        &mut self,
        content: &mut Option<String>,
        delimiter: &str,
        strip_tabs: bool,
    ) -> bool {
        loop {
            let mut line = String::new();
            let mut saw_newline = false;

            while let Some(ch) = self.peek_char() {
                self.advance();
                if ch == '\n' {
                    saw_newline = true;
                    break;
                }
                line.push(ch);
            }

            Self::push_capture_str(content, &line);
            if saw_newline {
                Self::push_capture_char(content, '\n');
            }

            if heredoc_line_matches_delimiter(&line, delimiter, strip_tabs) || !saw_newline {
                return true;
            }
        }
    }

    pub(in crate::parser) fn read_command_subst_into_depth(
        &mut self,
        content: &mut Option<String>,
        subst_depth: usize,
    ) -> bool {
        if subst_depth >= self.max_subst_depth {
            // Depth limit exceeded — consume until matching ')' and emit error token
            let mut depth = 1;
            while let Some(c) = self.peek_char() {
                self.advance();
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            Self::push_capture_char(content, ')');
                            return true;
                        }
                    }
                    _ => {}
                }
            }
            return false;
        }

        let mut depth = 1;
        let mut pending_heredocs = SmallVec::<[(String, bool); 2]>::new();
        let mut pending_case_headers = 0usize;
        let mut case_clause_depths = SmallVec::<[usize; 4]>::new();
        let mut current_word = String::with_capacity(16);
        let mut at_command_start = true;
        let mut expecting_redirection_target = false;
        let mut current_word_started_at_command_start = false;
        while let Some(c) = self.peek_char() {
            match c {
                '#' if !self.should_treat_hash_as_word_char() => {
                    let had_word = !current_word.is_empty();
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depths,
                        depth,
                        &mut current_word_started_at_command_start,
                    );
                    if had_word && expecting_redirection_target {
                        expecting_redirection_target = false;
                    }
                    Self::push_capture_char(content, '#');
                    self.advance();
                    while let Some(comment_ch) = self.peek_char() {
                        Self::push_capture_char(content, comment_ch);
                        self.advance();
                        if comment_ch == '\n' {
                            for (delimiter, strip_tabs) in pending_heredocs.drain(..) {
                                if !self.read_command_subst_pending_heredoc_into(
                                    content, &delimiter, strip_tabs,
                                ) {
                                    return false;
                                }
                            }
                            at_command_start = true;
                            expecting_redirection_target = false;
                            break;
                        }
                    }
                }
                '(' => {
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depths,
                        depth,
                        &mut current_word_started_at_command_start,
                    );
                    depth += 1;
                    Self::push_capture_char(content, c);
                    self.advance();
                    at_command_start = true;
                    expecting_redirection_target = false;
                }
                ')' => {
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depths,
                        depth,
                        &mut current_word_started_at_command_start,
                    );
                    if case_clause_depths
                        .last()
                        .is_some_and(|case_depth| *case_depth == depth)
                    {
                        Self::push_capture_char(content, ')');
                        self.advance();
                        at_command_start = true;
                        expecting_redirection_target = false;
                        continue;
                    }
                    depth -= 1;
                    self.advance();
                    if depth == 0 {
                        Self::push_capture_char(content, ')');
                        return true;
                    }
                    Self::push_capture_char(content, c);
                    at_command_start = false;
                    expecting_redirection_target = false;
                }
                '"' => {
                    let had_word = !current_word.is_empty();
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depths,
                        depth,
                        &mut current_word_started_at_command_start,
                    );
                    if had_word && expecting_redirection_target {
                        expecting_redirection_target = false;
                    }
                    // Nested double-quoted string inside $()
                    Self::push_capture_char(content, '"');
                    self.advance();
                    while let Some(qc) = self.peek_char() {
                        match qc {
                            '"' => {
                                Self::push_capture_char(content, '"');
                                self.advance();
                                break;
                            }
                            '\\' => {
                                Self::push_capture_char(content, '\\');
                                self.advance();
                                if let Some(esc) = self.peek_char() {
                                    Self::push_capture_char(content, esc);
                                    self.advance();
                                }
                            }
                            '$' => {
                                Self::push_capture_char(content, '$');
                                self.advance();
                                if self.peek_char() == Some('(') {
                                    if self.second_char() == Some('(') {
                                        if !self.read_arithmetic_expansion_into(content) {
                                            return false;
                                        }
                                    } else {
                                        Self::push_capture_char(content, '(');
                                        self.advance();
                                        if !self
                                            .read_command_subst_into_depth(content, subst_depth + 1)
                                        {
                                            return false;
                                        }
                                    }
                                }
                            }
                            _ => {
                                Self::push_capture_char(content, qc);
                                self.advance();
                            }
                        }
                    }
                    if expecting_redirection_target {
                        expecting_redirection_target = false;
                    } else {
                        at_command_start = false;
                    }
                }
                '\'' => {
                    let had_word = !current_word.is_empty();
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depths,
                        depth,
                        &mut current_word_started_at_command_start,
                    );
                    if had_word && expecting_redirection_target {
                        expecting_redirection_target = false;
                    }
                    // Single-quoted string inside $()
                    Self::push_capture_char(content, '\'');
                    self.advance();
                    while let Some(qc) = self.peek_char() {
                        Self::push_capture_char(content, qc);
                        self.advance();
                        if qc == '\'' {
                            break;
                        }
                    }
                    if expecting_redirection_target {
                        expecting_redirection_target = false;
                    } else {
                        at_command_start = false;
                    }
                }
                '`' => {
                    let had_word = !current_word.is_empty();
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depths,
                        depth,
                        &mut current_word_started_at_command_start,
                    );
                    if had_word && expecting_redirection_target {
                        expecting_redirection_target = false;
                    }
                    self.read_command_subst_backtick_segment_into(content);
                    if expecting_redirection_target {
                        expecting_redirection_target = false;
                    } else {
                        at_command_start = false;
                    }
                }
                '$' if self.second_char() == Some('\'') => {
                    let had_word = !current_word.is_empty();
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depths,
                        depth,
                        &mut current_word_started_at_command_start,
                    );
                    if had_word && expecting_redirection_target {
                        expecting_redirection_target = false;
                    }
                    Self::push_capture_char(content, '$');
                    self.advance();
                    Self::push_capture_char(content, '\'');
                    self.advance();
                    while let Some(qc) = self.peek_char() {
                        Self::push_capture_char(content, qc);
                        self.advance();
                        if qc == '\\' {
                            if let Some(esc) = self.peek_char() {
                                Self::push_capture_char(content, esc);
                                self.advance();
                            }
                            continue;
                        }
                        if qc == '\'' {
                            break;
                        }
                    }
                    if expecting_redirection_target {
                        expecting_redirection_target = false;
                    } else {
                        at_command_start = false;
                    }
                }
                '\\' => {
                    let had_word = !current_word.is_empty();
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depths,
                        depth,
                        &mut current_word_started_at_command_start,
                    );
                    if had_word && expecting_redirection_target {
                        expecting_redirection_target = false;
                    }
                    Self::push_capture_char(content, '\\');
                    self.advance();
                    if let Some(esc) = self.peek_char() {
                        Self::push_capture_char(content, esc);
                        self.advance();
                    }
                    if expecting_redirection_target {
                        expecting_redirection_target = false;
                    } else {
                        at_command_start = false;
                    }
                }
                '<' if self.second_char() == Some('<') => {
                    let word_was_redirection_fd = current_word_started_at_command_start
                        && !current_word.is_empty()
                        && current_word.chars().all(|current| current.is_ascii_digit());
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depths,
                        depth,
                        &mut current_word_started_at_command_start,
                    );
                    if word_was_redirection_fd {
                        at_command_start = true;
                    }

                    Self::push_capture_char(content, '<');
                    self.advance();
                    Self::push_capture_char(content, '<');
                    self.advance();

                    if self.peek_char() == Some('<') {
                        Self::push_capture_char(content, '<');
                        self.advance();
                        expecting_redirection_target = true;
                        continue;
                    }

                    let strip_tabs = if self.peek_char() == Some('-') {
                        Self::push_capture_char(content, '-');
                        self.advance();
                        true
                    } else {
                        false
                    };

                    if let Some(delimiter) = self.read_command_subst_heredoc_delimiter_into(content)
                    {
                        pending_heredocs.push((delimiter, strip_tabs));
                        expecting_redirection_target = false;
                    } else {
                        expecting_redirection_target = true;
                    }
                }
                '>' | '<' => {
                    let word_was_redirection_fd = current_word_started_at_command_start
                        && !current_word.is_empty()
                        && current_word.chars().all(|current| current.is_ascii_digit());
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depths,
                        depth,
                        &mut current_word_started_at_command_start,
                    );
                    if word_was_redirection_fd {
                        at_command_start = true;
                    }
                    Self::push_capture_char(content, c);
                    self.advance();
                    expecting_redirection_target = true;
                }
                '\n' => {
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depths,
                        depth,
                        &mut current_word_started_at_command_start,
                    );
                    Self::push_capture_char(content, '\n');
                    self.advance();
                    for (delimiter, strip_tabs) in pending_heredocs.drain(..) {
                        if !self.read_command_subst_pending_heredoc_into(
                            content, &delimiter, strip_tabs,
                        ) {
                            return false;
                        }
                    }
                    at_command_start = true;
                    expecting_redirection_target = false;
                }
                _ => {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        if current_word.is_empty()
                            && !expecting_redirection_target
                            && at_command_start
                        {
                            current_word_started_at_command_start = true;
                            at_command_start = false;
                        }
                        current_word.push(c);
                    } else {
                        let had_word = !current_word.is_empty();
                        Self::flush_command_subst_keyword(
                            &mut current_word,
                            &mut pending_case_headers,
                            &mut case_clause_depths,
                            depth,
                            &mut current_word_started_at_command_start,
                        );
                        if had_word && expecting_redirection_target {
                            expecting_redirection_target = false;
                        }
                        match c {
                            ' ' | '\t' => {}
                            ';' | '|' | '&' => {
                                at_command_start = true;
                                expecting_redirection_target = false;
                            }
                            _ => {
                                if !expecting_redirection_target {
                                    at_command_start = false;
                                }
                            }
                        }
                    }
                    Self::push_capture_char(content, c);
                    self.advance();
                }
            }
        }

        false
    }

    /// Read parameter expansion content after `${`, handling nested braces and quotes.
    /// In bash, quotes inside `${...}` (e.g. `${arr["key"]}`) don't terminate the
    /// outer double-quoted string. Appends chars including closing `}` to `content`.
    pub(in crate::parser) fn read_param_expansion_into(
        &mut self,
        content: &mut Option<String>,
        segment_start: Position,
    ) -> bool {
        let mut borrowable = true;
        let mut depth = 1;
        let mut literal_brace_depth = 0usize;
        let mut in_single = false;
        let mut in_double = false;
        let mut double_quote_depth = 0usize;
        while let Some(c) = self.peek_char() {
            if in_single {
                match c {
                    '\\' => {
                        let escape_start = self.current_position();
                        if self.second_char() == Some('"') {
                            self.advance();
                            borrowable = false;
                            self.ensure_capture_from_source(content, segment_start, escape_start);
                            Self::push_capture_char(content, '"');
                            self.advance();
                        } else {
                            Self::push_capture_char(content, '\\');
                            self.advance();
                        }
                    }
                    '\'' => {
                        Self::push_capture_char(content, c);
                        self.advance();
                        in_single = false;
                    }
                    _ => {
                        Self::push_capture_char(content, c);
                        self.advance();
                    }
                }
                continue;
            }

            match c {
                '}' if !in_single && (!in_double || depth > double_quote_depth) => {
                    self.advance();
                    Self::push_capture_char(content, '}');
                    if depth == 1
                        && literal_brace_depth > 0
                        && self.has_later_top_level_param_expansion_closer(depth)
                    {
                        literal_brace_depth -= 1;
                        continue;
                    }
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                '{' if !in_single && !in_double => {
                    literal_brace_depth += 1;
                    Self::push_capture_char(content, '{');
                    self.advance();
                }
                '"' => {
                    // Quotes inside ${...} are part of the expansion, not string delimiters
                    Self::push_capture_char(content, '"');
                    self.advance();
                    in_double = !in_double;
                    double_quote_depth = if in_double { depth } else { 0 };
                }
                '\'' => {
                    Self::push_capture_char(content, '\'');
                    self.advance();
                    if !in_double {
                        in_single = true;
                    }
                }
                '\\' => {
                    // Inside ${...} within double quotes, same escape rules apply:
                    // \", \\, \$, \` produce the escaped char; others keep backslash
                    let escape_start = self.current_position();
                    self.advance();
                    if let Some(esc) = self.peek_char() {
                        match esc {
                            '$' => {
                                borrowable = false;
                                self.ensure_capture_from_source(
                                    content,
                                    segment_start,
                                    escape_start,
                                );
                                Self::push_capture_char(content, '\x00');
                                Self::push_capture_char(content, '$');
                                self.advance();
                            }
                            '"' | '\\' | '`' => {
                                borrowable = false;
                                self.ensure_capture_from_source(
                                    content,
                                    segment_start,
                                    escape_start,
                                );
                                Self::push_capture_char(content, esc);
                                self.advance();
                            }
                            '}' => {
                                // \} should be a literal } without closing the expansion
                                Self::push_capture_char(content, '\\');
                                Self::push_capture_char(content, '}');
                                self.advance();
                                literal_brace_depth = literal_brace_depth.saturating_sub(1);
                            }
                            _ => {
                                Self::push_capture_char(content, '\\');
                                Self::push_capture_char(content, esc);
                                self.advance();
                            }
                        }
                    } else {
                        Self::push_capture_char(content, '\\');
                    }
                }
                '$' => {
                    Self::push_capture_char(content, '$');
                    self.advance();
                    if self.peek_char() == Some('(') {
                        if self.second_char() == Some('(') {
                            if !self.read_arithmetic_expansion_into(content) {
                                borrowable = false;
                            }
                        } else {
                            Self::push_capture_char(content, '(');
                            self.advance();
                            self.read_command_subst_into(content);
                        }
                    } else if self.peek_char() == Some('{') {
                        Self::push_capture_char(content, '{');
                        self.advance();
                        borrowable &= self.read_param_expansion_into(content, segment_start);
                    }
                }
                _ => {
                    Self::push_capture_char(content, c);
                    self.advance();
                }
            }
        }
        borrowable
    }

    pub(in crate::parser) fn has_later_top_level_param_expansion_closer(
        &self,
        target_depth: usize,
    ) -> bool {
        let mut chars = self.lookahead_chars().peekable();
        let mut depth = target_depth;
        let mut in_single = false;
        let mut in_double = false;
        let mut double_quote_depth = 0usize;

        while let Some(ch) = chars.next() {
            if in_single {
                match ch {
                    '\'' => in_single = false,
                    '\\' if chars.peek() == Some(&'"') => {
                        chars.next();
                    }
                    '\\' => {}
                    _ => {}
                }
                continue;
            }

            if in_double {
                match ch {
                    '"' => {
                        in_double = false;
                        double_quote_depth = 0;
                    }
                    '\\' => {
                        chars.next();
                    }
                    '$' if chars.peek() == Some(&'{') => {
                        chars.next();
                        depth += 1;
                    }
                    '}' if depth > double_quote_depth => {
                        depth -= 1;
                    }
                    _ => {}
                }
                continue;
            }

            match ch {
                '\n' if depth == target_depth => return false,
                '\'' => in_single = true,
                '"' => {
                    in_double = true;
                    double_quote_depth = depth;
                }
                '\\' => {
                    chars.next();
                }
                '$' if chars.peek() == Some(&'{') => {
                    chars.next();
                    depth += 1;
                }
                '}' => {
                    if depth == target_depth {
                        return true;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }

        false
    }
}

fn next_char_boundary(input: &str, index: usize) -> Option<(char, usize)> {
    let ch = input.get(index..)?.chars().next()?;
    Some((ch, index + ch.len_utf8()))
}

pub(in crate::parser) fn line_has_unclosed_double_paren(prefix: &str) -> bool {
    let mut index = 0usize;
    let mut depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut escaped = false;

    while let Some((ch, next_index)) = next_char_boundary(prefix, index) {
        let was_escaped = escaped;
        if ch == '\\' && !in_single {
            escaped = !escaped;
            index = next_index;
            continue;
        }
        escaped = false;

        match ch {
            '\'' if !in_double && !in_backtick && !was_escaped => in_single = !in_single,
            '"' if !in_single && !in_backtick && !was_escaped => in_double = !in_double,
            '`' if !in_single && !in_double && !was_escaped => in_backtick = !in_backtick,
            '(' if !in_single
                && !in_double
                && !in_backtick
                && !was_escaped
                && prefix[next_index..].starts_with('(') =>
            {
                depth += 1;
                index = next_index + '('.len_utf8();
                continue;
            }
            ')' if !in_single
                && !in_double
                && !in_backtick
                && !was_escaped
                && prefix[next_index..].starts_with(')') =>
            {
                depth = depth.saturating_sub(1);
                index = next_index + ')'.len_utf8();
                continue;
            }
            _ => {}
        }

        index = next_index;
    }

    depth > 0
}

fn inside_unclosed_double_paren_on_line(input: &str, index: usize) -> bool {
    let line_start = input[..index].rfind('\n').map_or(0, |found| found + 1);
    let prefix = &input[line_start..index];
    line_has_unclosed_double_paren(prefix)
}

pub(in crate::parser) fn hash_starts_comment(input: &str, index: usize) -> bool {
    if inside_unclosed_double_paren_on_line(input, index) {
        return false;
    }

    let next = &input[index + '#'.len_utf8()..];
    input[..index]
        .chars()
        .next_back()
        .is_none_or(|prev| match prev {
            '(' => {
                let whitespace_index = next.find(char::is_whitespace);
                let close_index = next.find(')');

                match (whitespace_index, close_index) {
                    (Some(whitespace), Some(close)) => whitespace < close,
                    (Some(_), None) | (None, None) => true,
                    (None, Some(_)) => false,
                }
            }
            _ => prev.is_whitespace() || matches!(prev, ';' | '|' | '&' | '<' | '>' | ')'),
        })
}

fn heredoc_delimiter_is_terminator(
    ch: char,
    in_single: bool,
    in_double: bool,
    escaped: bool,
) -> bool {
    !in_single
        && !in_double
        && !escaped
        && (ch.is_whitespace() || matches!(ch, '|' | '&' | ';' | '<' | '>' | '(' | ')'))
}

fn scan_double_quoted_command_substitution_segment(
    input: &str,
    mut index: usize,
    subst_depth: usize,
) -> Option<usize> {
    while let Some((ch, next_index)) = next_char_boundary(input, index) {
        match ch {
            '"' => return Some(next_index),
            '\\' => {
                index = next_index;
                if let Some((_, escaped_next)) = next_char_boundary(input, index) {
                    index = escaped_next;
                }
            }
            '$' if input[next_index..].starts_with('{') => {
                let consumed = scan_command_subst_parameter_expansion_len(
                    &input[next_index + '{'.len_utf8()..],
                    subst_depth,
                    0,
                )?;
                index = next_index + '{'.len_utf8() + consumed;
            }
            '$' if input[next_index..].starts_with('(')
                && !input[next_index + '('.len_utf8()..].starts_with('(') =>
            {
                let consumed = scan_command_substitution_body_len_inner(
                    &input[next_index + '('.len_utf8()..],
                    subst_depth + 1,
                )?;
                index = next_index + '('.len_utf8() + consumed;
            }
            _ => index = next_index,
        }
    }

    None
}

fn scan_command_subst_parameter_expansion_len(
    input: &str,
    subst_depth: usize,
    parameter_depth: usize,
) -> Option<usize> {
    if parameter_depth >= MAX_PARAMETER_EXPANSION_SCAN_DEPTH {
        return scan_command_subst_parameter_expansion_len_balanced(input, subst_depth);
    }

    let mut index = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_ansi_c_single = false;
    let mut in_backtick = false;
    let mut escaped = false;
    let mut ansi_c_quote_pending = false;

    while let Some((ch, next_index)) = next_char_boundary(input, index) {
        let was_escaped = escaped;
        if ch == '\\' && !in_single {
            escaped = !escaped;
            index = next_index;
            ansi_c_quote_pending = false;
            continue;
        }
        escaped = false;

        if !in_single && !in_ansi_c_single && !in_backtick && !was_escaped && ch == '$' {
            if input[next_index..].starts_with('{')
                && let Some(consumed) = scan_command_subst_parameter_expansion_len(
                    &input[next_index + '{'.len_utf8()..],
                    subst_depth,
                    parameter_depth + 1,
                )
            {
                index = next_index + '{'.len_utf8() + consumed;
                ansi_c_quote_pending = false;
                continue;
            }

            if input[next_index..].starts_with('(')
                && !input[next_index + '('.len_utf8()..].starts_with('(')
                && let Some(consumed) = scan_command_substitution_body_len_inner(
                    &input[next_index + '('.len_utf8()..],
                    subst_depth + 1,
                )
            {
                index = next_index + '('.len_utf8() + consumed;
                ansi_c_quote_pending = false;
                continue;
            }
        }

        if !in_single
            && !in_ansi_c_single
            && !in_double
            && !in_backtick
            && !was_escaped
            && matches!(ch, '<' | '>')
            && input[next_index..].starts_with('(')
            && let Some(consumed) = scan_command_substitution_body_len_inner(
                &input[next_index + '('.len_utf8()..],
                subst_depth + 1,
            )
        {
            index = next_index + '('.len_utf8() + consumed;
            ansi_c_quote_pending = false;
            continue;
        }

        match ch {
            '\'' if !in_double && !in_backtick && !was_escaped => {
                if in_ansi_c_single {
                    in_ansi_c_single = false;
                } else if !in_single && ansi_c_quote_pending {
                    in_ansi_c_single = true;
                } else {
                    in_single = !in_single;
                }
            }
            '"' if !in_single && !in_ansi_c_single && !in_backtick && !was_escaped => {
                in_double = !in_double
            }
            '`' if !in_single && !in_ansi_c_single && !in_double && !was_escaped => {
                in_backtick = !in_backtick
            }
            '}' if !in_single
                && !in_ansi_c_single
                && !in_double
                && !in_backtick
                && !was_escaped =>
            {
                return Some(next_index);
            }
            _ => {}
        }

        ansi_c_quote_pending = ch == '$'
            && !in_single
            && !in_ansi_c_single
            && !in_double
            && !in_backtick
            && !was_escaped;
        index = next_index;
    }

    None
}

fn scan_command_subst_parameter_expansion_len_balanced(
    input: &str,
    subst_depth: usize,
) -> Option<usize> {
    let mut index = 0usize;
    let mut brace_depth = 1usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_ansi_c_single = false;
    let mut in_backtick = false;
    let mut escaped = false;
    let mut ansi_c_quote_pending = false;

    while let Some((ch, next_index)) = next_char_boundary(input, index) {
        let was_escaped = escaped;
        if ch == '\\' && !in_single {
            escaped = !escaped;
            index = next_index;
            ansi_c_quote_pending = false;
            continue;
        }
        escaped = false;

        if !in_single && !in_ansi_c_single && !in_backtick && !was_escaped && ch == '$' {
            if input[next_index..].starts_with('{') {
                brace_depth = brace_depth.saturating_add(1);
                index = next_index + '{'.len_utf8();
                ansi_c_quote_pending = false;
                continue;
            }

            if input[next_index..].starts_with('(')
                && !input[next_index + '('.len_utf8()..].starts_with('(')
                && let Some(consumed) = scan_command_substitution_body_len_inner(
                    &input[next_index + '('.len_utf8()..],
                    subst_depth + 1,
                )
            {
                index = next_index + '('.len_utf8() + consumed;
                ansi_c_quote_pending = false;
                continue;
            }
        }

        if !in_single
            && !in_ansi_c_single
            && !in_double
            && !in_backtick
            && !was_escaped
            && matches!(ch, '<' | '>')
            && input[next_index..].starts_with('(')
            && let Some(consumed) = scan_command_substitution_body_len_inner(
                &input[next_index + '('.len_utf8()..],
                subst_depth + 1,
            )
        {
            index = next_index + '('.len_utf8() + consumed;
            ansi_c_quote_pending = false;
            continue;
        }

        match ch {
            '\'' if !in_double && !in_backtick && !was_escaped => {
                if in_ansi_c_single {
                    in_ansi_c_single = false;
                } else if !in_single && ansi_c_quote_pending {
                    in_ansi_c_single = true;
                } else {
                    in_single = !in_single;
                }
            }
            '"' if !in_single && !in_ansi_c_single && !in_backtick && !was_escaped => {
                in_double = !in_double
            }
            '`' if !in_single && !in_ansi_c_single && !in_double && !was_escaped => {
                in_backtick = !in_backtick
            }
            '}' if !in_single
                && !in_ansi_c_single
                && !in_double
                && !in_backtick
                && !was_escaped =>
            {
                brace_depth = brace_depth.saturating_sub(1);
                if brace_depth == 0 {
                    return Some(next_index);
                }
            }
            _ => {}
        }

        ansi_c_quote_pending = ch == '$'
            && !in_single
            && !in_ansi_c_single
            && !in_double
            && !in_backtick
            && !was_escaped;
        index = next_index;
    }

    None
}

fn scan_command_subst_heredoc_delimiter(input: &str, mut index: usize) -> Option<(usize, String)> {
    while let Some((ch, next_index)) = next_char_boundary(input, index) {
        if !matches!(ch, ' ' | '\t') {
            break;
        }
        index = next_index;
    }

    let start = index;
    let mut cooked = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    while let Some((ch, next_index)) = next_char_boundary(input, index) {
        if heredoc_delimiter_is_terminator(ch, in_single, in_double, escaped) {
            break;
        }

        index = next_index;
        if escaped {
            cooked.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ => cooked.push(ch),
        }
    }

    (index > start).then_some((index, cooked))
}

fn skip_command_subst_pending_heredoc(
    input: &str,
    mut index: usize,
    delimiter: &str,
    strip_tabs: bool,
) -> usize {
    while index <= input.len() {
        let rest = &input[index..];
        let line_len = rest.find('\n').unwrap_or(rest.len());
        let line = &rest[..line_len];
        let has_newline = line_len < rest.len();

        index += line_len;
        if has_newline {
            index += '\n'.len_utf8();
        }

        if heredoc_line_matches_delimiter(line, delimiter, strip_tabs) || !has_newline {
            return index;
        }
    }

    index
}

fn scan_command_subst_ansi_c_single_quoted_segment(
    input: &str,
    quote_index: usize,
) -> Option<usize> {
    let mut index = quote_index + '\''.len_utf8();

    while let Some((ch, next_index)) = next_char_boundary(input, index) {
        index = next_index;
        if ch == '\\' {
            if let Some((_, escaped_next)) = next_char_boundary(input, index) {
                index = escaped_next;
            }
            continue;
        }

        if ch == '\'' {
            return Some(index);
        }
    }

    None
}

fn scan_command_subst_backtick_segment(input: &str, start: usize) -> Option<usize> {
    let mut index = start;

    while let Some((ch, next_index)) = next_char_boundary(input, index) {
        index = next_index;
        if ch == '\\' {
            if let Some((_, escaped_next)) = next_char_boundary(input, index) {
                index = escaped_next;
            }
            continue;
        }

        if ch == '`' {
            return Some(index);
        }
    }

    None
}

fn flush_scanned_command_subst_keyword(
    current_word: &mut String,
    pending_case_headers: &mut usize,
    case_clause_depths: &mut SmallVec<[usize; 4]>,
    depth: usize,
    word_started_at_command_start: &mut bool,
) {
    if current_word.is_empty() {
        *word_started_at_command_start = false;
        return;
    }

    match current_word.as_str() {
        "case" if *word_started_at_command_start => *pending_case_headers += 1,
        "in" if *pending_case_headers > 0 => {
            *pending_case_headers -= 1;
            case_clause_depths.push(depth);
        }
        "esac" if *word_started_at_command_start => {
            case_clause_depths.pop();
        }
        _ => {}
    }

    current_word.clear();
    *word_started_at_command_start = false;
}

pub(in crate::parser) fn scan_command_substitution_body_len_inner(
    input: &str,
    subst_depth: usize,
) -> Option<usize> {
    if subst_depth >= DEFAULT_MAX_SUBST_DEPTH {
        return None;
    }

    let mut index = 0usize;
    let mut depth = 1;
    let mut pending_heredocs = SmallVec::<[(String, bool); 2]>::new();
    let mut pending_case_headers = 0usize;
    let mut case_clause_depths = SmallVec::<[usize; 4]>::new();
    let mut current_word = String::with_capacity(16);
    let mut at_command_start = true;
    let mut expecting_redirection_target = false;
    let mut current_word_started_at_command_start = false;

    while let Some((ch, next_index)) = next_char_boundary(input, index) {
        match ch {
            '#' if hash_starts_comment(input, index) => {
                let had_word = !current_word.is_empty();
                flush_scanned_command_subst_keyword(
                    &mut current_word,
                    &mut pending_case_headers,
                    &mut case_clause_depths,
                    depth,
                    &mut current_word_started_at_command_start,
                );
                if had_word && expecting_redirection_target {
                    expecting_redirection_target = false;
                }
                index = next_index;
                while let Some((comment_ch, comment_next)) = next_char_boundary(input, index) {
                    index = comment_next;
                    if comment_ch == '\n' {
                        for (delimiter, strip_tabs) in pending_heredocs.drain(..) {
                            index = skip_command_subst_pending_heredoc(
                                input, index, &delimiter, strip_tabs,
                            );
                        }
                        at_command_start = true;
                        expecting_redirection_target = false;
                        break;
                    }
                }
            }
            '(' => {
                flush_scanned_command_subst_keyword(
                    &mut current_word,
                    &mut pending_case_headers,
                    &mut case_clause_depths,
                    depth,
                    &mut current_word_started_at_command_start,
                );
                depth += 1;
                index = next_index;
                at_command_start = true;
                expecting_redirection_target = false;
            }
            ')' => {
                flush_scanned_command_subst_keyword(
                    &mut current_word,
                    &mut pending_case_headers,
                    &mut case_clause_depths,
                    depth,
                    &mut current_word_started_at_command_start,
                );
                if case_clause_depths
                    .last()
                    .is_some_and(|case_depth| *case_depth == depth)
                {
                    index = next_index;
                    at_command_start = true;
                    expecting_redirection_target = false;
                    continue;
                }
                depth -= 1;
                index = next_index;
                if depth == 0 {
                    return Some(index);
                }
                at_command_start = false;
                expecting_redirection_target = false;
            }
            '"' => {
                let had_word = !current_word.is_empty();
                flush_scanned_command_subst_keyword(
                    &mut current_word,
                    &mut pending_case_headers,
                    &mut case_clause_depths,
                    depth,
                    &mut current_word_started_at_command_start,
                );
                if had_word && expecting_redirection_target {
                    expecting_redirection_target = false;
                }
                index = scan_double_quoted_command_substitution_segment(
                    input,
                    next_index,
                    subst_depth,
                )?;
                if expecting_redirection_target {
                    expecting_redirection_target = false;
                } else {
                    at_command_start = false;
                }
            }
            '\'' => {
                let had_word = !current_word.is_empty();
                flush_scanned_command_subst_keyword(
                    &mut current_word,
                    &mut pending_case_headers,
                    &mut case_clause_depths,
                    depth,
                    &mut current_word_started_at_command_start,
                );
                if had_word && expecting_redirection_target {
                    expecting_redirection_target = false;
                }
                index = next_index;
                while let Some((quoted_ch, quoted_next)) = next_char_boundary(input, index) {
                    index = quoted_next;
                    if quoted_ch == '\'' {
                        break;
                    }
                }
                if expecting_redirection_target {
                    expecting_redirection_target = false;
                } else {
                    at_command_start = false;
                }
            }
            '`' => {
                let had_word = !current_word.is_empty();
                flush_scanned_command_subst_keyword(
                    &mut current_word,
                    &mut pending_case_headers,
                    &mut case_clause_depths,
                    depth,
                    &mut current_word_started_at_command_start,
                );
                if had_word && expecting_redirection_target {
                    expecting_redirection_target = false;
                }
                index = scan_command_subst_backtick_segment(input, next_index)?;
                if expecting_redirection_target {
                    expecting_redirection_target = false;
                } else {
                    at_command_start = false;
                }
            }
            '$' if input[next_index..].starts_with('\'') => {
                let had_word = !current_word.is_empty();
                flush_scanned_command_subst_keyword(
                    &mut current_word,
                    &mut pending_case_headers,
                    &mut case_clause_depths,
                    depth,
                    &mut current_word_started_at_command_start,
                );
                if had_word && expecting_redirection_target {
                    expecting_redirection_target = false;
                }
                index = scan_command_subst_ansi_c_single_quoted_segment(input, next_index)?;
                if expecting_redirection_target {
                    expecting_redirection_target = false;
                } else {
                    at_command_start = false;
                }
            }
            '\\' => {
                let had_word = !current_word.is_empty();
                flush_scanned_command_subst_keyword(
                    &mut current_word,
                    &mut pending_case_headers,
                    &mut case_clause_depths,
                    depth,
                    &mut current_word_started_at_command_start,
                );
                if had_word && expecting_redirection_target {
                    expecting_redirection_target = false;
                }
                index = next_index;
                if let Some((_, escaped_next)) = next_char_boundary(input, index) {
                    index = escaped_next;
                }
                if expecting_redirection_target {
                    expecting_redirection_target = false;
                } else {
                    at_command_start = false;
                }
            }
            '>' => {
                let word_was_redirection_fd = current_word_started_at_command_start
                    && !current_word.is_empty()
                    && current_word.chars().all(|current| current.is_ascii_digit());
                flush_scanned_command_subst_keyword(
                    &mut current_word,
                    &mut pending_case_headers,
                    &mut case_clause_depths,
                    depth,
                    &mut current_word_started_at_command_start,
                );
                if word_was_redirection_fd {
                    at_command_start = true;
                }
                index = next_index;
                expecting_redirection_target = true;
            }
            '<' if input[next_index..].starts_with('<') => {
                let word_was_redirection_fd = current_word_started_at_command_start
                    && !current_word.is_empty()
                    && current_word.chars().all(|current| current.is_ascii_digit());
                let had_word = !current_word.is_empty();
                flush_scanned_command_subst_keyword(
                    &mut current_word,
                    &mut pending_case_headers,
                    &mut case_clause_depths,
                    depth,
                    &mut current_word_started_at_command_start,
                );
                if had_word && expecting_redirection_target {
                    expecting_redirection_target = false;
                }
                if word_was_redirection_fd {
                    at_command_start = true;
                }
                if inside_unclosed_double_paren_on_line(input, index) {
                    index = next_index + '<'.len_utf8();
                    continue;
                }

                if input[next_index + '<'.len_utf8()..].starts_with('<') {
                    index = next_index + '<'.len_utf8() + '<'.len_utf8();
                    expecting_redirection_target = true;
                    continue;
                }

                let strip_tabs = input[next_index..].starts_with("<-");
                let delimiter_start = next_index + if strip_tabs { 2 } else { 1 };
                if let Some((delimiter_index, delimiter)) =
                    scan_command_subst_heredoc_delimiter(input, delimiter_start)
                {
                    pending_heredocs.push((delimiter, strip_tabs));
                    index = delimiter_index;
                    expecting_redirection_target = false;
                } else {
                    index = next_index;
                    expecting_redirection_target = true;
                }
            }
            '\n' => {
                flush_scanned_command_subst_keyword(
                    &mut current_word,
                    &mut pending_case_headers,
                    &mut case_clause_depths,
                    depth,
                    &mut current_word_started_at_command_start,
                );
                index = next_index;
                for (delimiter, strip_tabs) in pending_heredocs.drain(..) {
                    index =
                        skip_command_subst_pending_heredoc(input, index, &delimiter, strip_tabs);
                }
                at_command_start = true;
                expecting_redirection_target = false;
            }
            '$' if input[next_index..].starts_with('{') => {
                let had_word = !current_word.is_empty();
                flush_scanned_command_subst_keyword(
                    &mut current_word,
                    &mut pending_case_headers,
                    &mut case_clause_depths,
                    depth,
                    &mut current_word_started_at_command_start,
                );
                if had_word && expecting_redirection_target {
                    expecting_redirection_target = false;
                }
                let consumed = scan_command_subst_parameter_expansion_len(
                    &input[next_index + '{'.len_utf8()..],
                    subst_depth,
                    0,
                )?;
                index = next_index + '{'.len_utf8() + consumed;
                if expecting_redirection_target {
                    expecting_redirection_target = false;
                } else {
                    at_command_start = false;
                }
            }
            '$' if input[next_index..].starts_with('(')
                && !input[next_index + '('.len_utf8()..].starts_with('(') =>
            {
                let had_word = !current_word.is_empty();
                flush_scanned_command_subst_keyword(
                    &mut current_word,
                    &mut pending_case_headers,
                    &mut case_clause_depths,
                    depth,
                    &mut current_word_started_at_command_start,
                );
                if had_word && expecting_redirection_target {
                    expecting_redirection_target = false;
                }
                let consumed = scan_command_substitution_body_len_inner(
                    &input[next_index + '('.len_utf8()..],
                    subst_depth + 1,
                )?;
                index = next_index + '('.len_utf8() + consumed;
                if expecting_redirection_target {
                    expecting_redirection_target = false;
                } else {
                    at_command_start = false;
                }
            }
            _ => {
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    if current_word.is_empty() && !expecting_redirection_target && at_command_start
                    {
                        current_word_started_at_command_start = true;
                        at_command_start = false;
                    }
                    current_word.push(ch);
                } else {
                    let had_word = !current_word.is_empty();
                    flush_scanned_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depths,
                        depth,
                        &mut current_word_started_at_command_start,
                    );
                    if had_word && expecting_redirection_target {
                        expecting_redirection_target = false;
                    }
                    match ch {
                        ' ' | '\t' => {}
                        ';' | '|' | '&' => {
                            at_command_start = true;
                            expecting_redirection_target = false;
                        }
                        _ => {
                            if !expecting_redirection_target {
                                at_command_start = false;
                            }
                        }
                    }
                }
                index = next_index;
            }
        }
    }

    None
}

pub(in crate::parser) fn scan_command_substitution_body_len(input: &str) -> Option<usize> {
    scan_command_substitution_body_len_inner(input, 0)
}
