# OILS Parser Checklist (Non-OILS/YSH)

Analyzed `crates/shuck-parser/tests/testdata/oils_expectations.json` on 2026-04-04, ignoring entries whose reason is `case uses YSH/OILS-only syntax or option modes outside the current Bash parser`.

Summary:
- 35 actionable `skip` entries remain in scope.
- 9 formerly skipped cases already parse and have now been removed from `oils_expectations.json`.

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

- [ ] `oils/command-sub.test.sh::Here doc with pipeline` - prefix heredoc must attach to `tac` and still allow the following pipeline.
- [ ] `oils/here-doc.test.sh::Function def and execution with here doc` - allow a trailing heredoc redirect after a function definition.
- [ ] `oils/here-doc.test.sh::Here doc and < redirect -- last one wins` - trailing `< file` after a heredoc must be collected, not left as a new top-level command.
- [ ] `oils/here-doc.test.sh::Here doc as command prefix` - allow `<<EOF cmd` with no command word before the redirect.
- [ ] `oils/here-doc.test.sh::Redirect after here doc` - collect `1>&2` and other fd-dup redirects after a heredoc, not just a subset of redirect forms.
- [ ] `oils/here-doc.test.sh::Two here docs -- first is ignored; second ones wins!` - keep parsing after the first prefix heredoc so both heredocs attach to the same command.
- [ ] `oils/redirect-command.test.sh::>$file touches a file` - accept redirect-only commands like `>file`.
- [ ] `oils/redirect-command.test.sh::< file in pipeline and subshell doesn't work` - accept redirect-only commands as pipeline stages and inside subshells.
- [ ] `oils/redirect-command.test.sh::Redirect in function body` - accept a trailing redirect after a function definition.
- [ ] `oils/redirect-command.test.sh::Redirect in function body AND function call` - same trailing-function-redirect fix; the later call-site redirect already fits once definition parsing works.
- [ ] `oils/redirect-command.test.sh::Redirect in function body is evaluated multiple times` - same parser fix; runtime semantics are separate.
- [ ] `oils/redirect-multi.test.sh::Redirect to $empty (in function body)` - same trailing-function-redirect fix for word targets that expand at runtime.
- [ ] `oils/redirect.test.sh::2>&1 with no command` - accept redirect-only commands consisting only of fd duplication.
- [ ] `oils/toysh.test.sh::{abc}<<< - http://landley.net/notes-2019.html#09-12-2019` - allow `{fdvar}` redirects after compound commands, not only after simple commands.
- [ ] `oils/var-sub.test.sh::Here doc with bad "$@" delimiter` - validate heredoc delimiters as static literal words before reading the body; reject dynamic delimiters like `"$@"` up front.
- [ ] `oils/vars-special.test.sh::$LINENO in "bare" redirect arg (bug regression)` - same redirect-only command support; `> $TMP/bare$LINENO` should parse even when the operator is separated by whitespace.

## Missing Redirect Operators

Shared work: extend lexer token coverage for `|&`, `&>>`, and `<>`. Parser should lower `|&` to pipe plus `2>&1` on the left-hand command, and can lower `&>>` to existing append-plus-dup redirects if we do not want a dedicated AST node. AST change: add `RedirectKind::ReadWrite` or equivalent for `<>`; AST change for `&>>` is optional and only needed for source fidelity.

- [ ] `oils/pipeline.test.sh::|&` - recognize the operator and lower it correctly.
- [ ] `oils/redirect.test.sh::&>> appends stdout and stderr` - recognize `&>>` as append-both, not `&>` followed by `>`.
- [ ] `oils/redirect.test.sh::<> for read/write` - add read-write redirect support.
- [ ] `oils/redirect.test.sh::<> for read/write named pipes` - same `<>` support on named pipes.
- [ ] `oils/redirect.test.sh::Named read-write file descriptor` - same `<>` support plus existing `{fdvar}` plumbing.
- [ ] `oils/redirect.test.sh::noclobber can still write to non-regular files like /dev/null` - this script stays blocked until `&>>` is parsed.

## Nested Shell Constructs, Function Bodies, and (( Ambiguity)

Shared work: parser needs to distinguish arithmetic `(( ... ))` from grouped-command or subshell spellings that happen to start with `((`. Function definitions should accept any compound command body, not just `{ ... }`. The lexer's `$(` scanner should stop terminating a command substitution on case-item `)` tokens. AST change: none.

- [ ] `oils/command-sub.test.sh::case in subshell` - `$(` scanning must survive `case ... pattern)` inside the substitution body.
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
