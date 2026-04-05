# OILS Parser Checklist (Non-OILS/YSH)

Analyzed `crates/shuck-parser/tests/testdata/oils_expectations.json` on 2026-04-04, ignoring entries whose reason is `case uses YSH/OILS-only syntax or option modes outside the current Bash parser`.

Summary:
- 12 actionable `skip` entries remain in scope.
- 31 formerly skipped cases already parse and have now been removed from `oils_expectations.json`.
- 1 formerly skipped case now intentionally fails parsing and has been reclassified as `parse_err`.

## Expectation Cleanup (Already Parses)

Shared work: expectations/tests only. No lexer, AST, or parser changes needed.

- [x] `oils/regex.test.sh::Multiple adjacent () groups` - current parser returns `OK`; removed the skip entry from `oils_expectations.json`.
- [x] `oils/var-sub.test.sh::Braced block inside ${}` - current parser returns `OK`; removed the skip entry from `oils_expectations.json`.

## Alias Expansion

Shared work: add a Bash-like alias expansion and token reinjection phase at the lexer/preparse boundary, then reparse alias-expanded text so injected `for`, `{`, `(`, `do`, and `)` are seen as structural tokens. AST change: none.

- [x] `oils/alias.test.sh::Loop split across alias in another way` - added parser-side alias expansion with synthetic token injection so alias values can complete a `for ... do ... done` header.
- [x] `oils/alias.test.sh::Loop split across both iterative and recursive aliases` - recursive alias chains now expand through the parser’s synthetic token queue, including trailing-space re-expansion of subsequent words.
- [x] `oils/alias.test.sh::alias for left brace` - alias expansion now allows an alias to produce `{` so brace groups parse as compound commands.
- [x] `oils/alias.test.sh::alias for left paren` - alias expansion now allows an alias to produce `(` so subshells parse as compound commands.

## Indexed Assignment and Arithmetic Header Lexing

Shared work: make assignment detection more grammar-aware. The lexer and parser should keep `name[expr]=value` together even when the subscript contains spaces or parentheses, and function-definition detection should only fire on a bare identifier followed by `()`. Arithmetic `for` headers need a mode where `<(` is less-than plus `(`, not process substitution. AST change: none.

- [x] `oils/ble-idioms.test.sh::Issue #1069 [53] - LHS array parsing a[1 + 2]=3 (see spec/array-assign for more)` - parser-side indexed-assignment scanning now reconstructs `name[expr]=value` across split tokens, and POSIX function detection only triggers on bare identifiers.
- [x] `oils/bugs.test.sh::for loop (issue #1446)` - arithmetic parsing now treats `ProcessSubIn/Out` tokens as `<` or `>` plus `(` in arithmetic contexts, so `n<(3-(1))` stays inside the header.
- [x] `oils/bugs.test.sh::for loop 2 (issue #1446)` - the same arithmetic-context handling now covers the spaced `3- (1)` variant as well.

## Redirect Placement, Redirect-Only Commands, and Heredocs

Shared work: parser should support redirect-only simple commands, redirect prefixes before the command name, multiple heredocs on the same command, and full trailing redirect collection after heredocs and compound commands. Function definitions need trailing redirects attached to the function body command. AST change: none with the current `Command::Compound(..., redirects)` plus empty-name `SimpleCommand`.

- [x] `oils/command-sub.test.sh::Here doc with pipeline` - heredoc replay now preserves only same-line tail tokens, so the prefix heredoc still attaches to `tac` and the following pipeline remains visible.
- [x] `oils/here-doc.test.sh::Function def and execution with here doc` - function definitions now absorb trailing heredoc redirects on the function body command.
- [x] `oils/here-doc.test.sh::Here doc and < redirect -- last one wins` - trailing `< file` redirects are now collected after a heredoc instead of becoming a new top-level command.
- [x] `oils/here-doc.test.sh::Here doc as command prefix` - prefix heredocs now work before the command name by replaying the rest of the original command line after the heredoc body.
- [x] `oils/here-doc.test.sh::Redirect after here doc` - trailing fd-dup and related redirect forms are now collected after a heredoc.
- [x] `oils/here-doc.test.sh::Two here docs -- first is ignored; second ones wins!` - multiple same-line heredocs now stay attached to one command without consuming the next line’s keywords.
- [x] `oils/redirect-command.test.sh::>$file touches a file` - redirect-only commands now parse as empty-name simple commands.
- [x] `oils/redirect-command.test.sh::< file in pipeline and subshell doesn't work` - redirect-only commands now parse inside pipelines and subshells too.
- [x] `oils/redirect-command.test.sh::Redirect in function body` - trailing redirects now attach correctly to function-definition bodies.
- [x] `oils/redirect-command.test.sh::Redirect in function body AND function call` - the same function-body redirect handling now covers the mixed definition-and-call form.
- [x] `oils/redirect-command.test.sh::Redirect in function body is evaluated multiple times` - the parser now accepts the function-body redirect form; runtime semantics are unchanged.
- [x] `oils/redirect-multi.test.sh::Redirect to $empty (in function body)` - the same trailing function-body redirect parsing now works with runtime-expanded targets.
- [x] `oils/redirect.test.sh::2>&1 with no command` - redirect-only fd-dup commands now parse with an empty command name.
- [x] `oils/toysh.test.sh::{abc}<<< - http://landley.net/notes-2019.html#09-12-2019` - `{fdvar}` redirects now parse after compound commands, not just after simple commands.
- [x] `oils/var-sub.test.sh::Here doc with bad "$@" delimiter` - heredoc delimiters now must be static literal words, so dynamic delimiters are rejected up front and tracked as `parse_err`.
- [x] `oils/vars-special.test.sh::$LINENO in "bare" redirect arg (bug regression)` - spaced bare redirects now parse even when the target word contains expansions like `$LINENO`.

## Missing Redirect Operators

Shared work: extend lexer token coverage for `|&`, `&>>`, and `<>`. Parser should lower `|&` to pipe plus `2>&1` on the left-hand command, and can lower `&>>` to existing append-plus-dup redirects if we do not want a dedicated AST node. AST change: add `RedirectKind::ReadWrite` or equivalent for `<>`; AST change for `&>>` is optional and only needed for source fidelity.

- [x] `oils/pipeline.test.sh::|&` - the lexer now recognizes `|&` as a dedicated pipeline operator, and the parser lowers it by appending `2>&1` to the left-hand command before building the pipeline.
- [x] `oils/redirect.test.sh::&>> appends stdout and stderr` - the lexer now recognizes `&>>`, and the parser lowers it to `>> file` plus `2>&1` so append-both redirection parses without a new AST node.
- [x] `oils/redirect.test.sh::<> for read/write` - the lexer and parser now recognize plain `<>` and lower it to a dedicated `RedirectKind::ReadWrite`.
- [x] `oils/redirect.test.sh::<> for read/write named pipes` - the same read-write redirect support now covers named-pipe operands too.
- [x] `oils/redirect.test.sh::Named read-write file descriptor` - numeric and `{fdvar}` read-write redirects now parse through the existing fd-variable plumbing.
- [x] `oils/redirect.test.sh::noclobber can still write to non-regular files like /dev/null` - once `&>>` parses as append-both redirection, the `/dev/null` noclobber case is no longer blocked at parse time.

## Nested Shell Constructs, Function Bodies, and (( Ambiguity)

Shared work: parser needs to distinguish arithmetic `(( ... ))` from grouped-command or subshell spellings that happen to start with `((`. Function definitions should accept any compound command body, not just `{ ... }`. The lexer's `$(` scanner should stop terminating a command substitution on case-item `)` tokens. AST change: none.

- [x] `oils/command-sub.test.sh::case in subshell` - the `$(` scanner now keeps top-level case-item `)` tokens inside an open `case ... in ... esac`, so command substitutions are not terminated early.
- [ ] `oils/divergence.test.sh::builtin cat crashes a subshell (#2530)` - treat `(( ... ) | true)` here as grouped commands or subshell syntax, not an arithmetic command.
- [ ] `oils/for-expr.test.sh::Accepts { } syntax too` - allow `{ ... }` as the body form after an arithmetic `for ((...))` header if we want this Bash-compatible extension.
- [ ] `oils/func-parsing.test.sh::subshell function` - allow `name() ( ... )` as a function body.
- [ ] `oils/paren-ambiguity.test.sh::(( closed with ) ) after multiple lines is command - #2337` - same `((` ambiguity resolution; this is not an arithmetic command.
- [ ] `oils/paren-ambiguity.test.sh::((test example - liblo package - #2337` - same `((` ambiguity resolution when `((` is immediately followed by `test`.
- [ ] `oils/print-source-code.test.sh::non-{ } function bodies can be serialized (rare)` - parser must first accept non-brace function bodies such as subshell bodies.
- [ ] `oils/sh-func.test.sh::Subshell function` - same `name() ( ... )` function-body support.

## Conditional and Glob Token Context

Shared work: lexer needs command-position awareness so tokens like `[[` and `{` are only structural where Bash treats them as keywords or operators. Glob and bracket-expression scanning also needs to keep complex glob words together instead of opening quotes or conditional syntax mid-word. Parser can treat leftover structural tokens as literal operands inside `[[ ... =~ ... ]]`. AST change: none.

- [ ] `oils/glob.test.sh::Glob of unescaped [[] and []]` - do not lex `[[` as a conditional opener when it is just glob text.
- [ ] `oils/regex.test.sh::Quoted { and +` - treat bare `{` as a literal operand inside `[[ ... =~ ... ]]`, not as a brace-group opener.
- [ ] `oils/toysh.test.sh::char class / extglob` - keep patterns like `[hello"]"`, `[$(echo abc)]`, `[+()]`, and `[+(])` as words instead of producing unterminated-quote errors.

## Parameter Expansion and Brace Operand Scanning

Shared work: both lexer-side `${...}` scanning and parser-side brace-operand reading need quote-aware, escape-aware brace tracking. A literal `}` inside `\}`, `'}` or `"}` must not close the expansion early. AST change: none.

- [ ] `oils/var-op-strip.test.sh::Strip Right Brace (#702)` - fix `${var#...}` operand scanning so quoted or escaped `}` stays inside the pattern.
- [ ] `oils/var-sub-quote.test.sh::Right Brace as argument (similar to #702)` - same fix for `${var-...}` operands inside quotes.
