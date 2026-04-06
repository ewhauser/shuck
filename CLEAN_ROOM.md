# AGENTS.md

## Clean-Room Policy

This project is a clean-room reimplementation of ShellCheck. To preserve the integrity of the independent authoring process, the following rules **must** be followed at all times.

### Prohibited Inputs

- **Do not** read, reference, or import ShellCheck source code.
- **Do not** read, reference, or import ShellCheck wiki pages or documentation examples.
- **Do not** reuse diagnostic wording from ShellCheck materials.
- **Do not** copy raw ShellCheck output into committed repository files.
- **Do not** search the web for ShellCheck source, wiki content, or diagnostic text.

### Approved Inputs

- Shell language manuals, specifications (POSIX, Bash reference manual), and semantic notes.
- Files already authored inside this repository.
- The ShellCheck binary used only as a **black-box oracle** (run it, observe numeric exit/code, but do not copy its output text into committed files). You can reverse engineer behavior by running code through the ShellCheck binary.
- The companion `shell-checks` repository (`../shell-checks`) which contains independently authored rule specs, examples, and compatibility mappings.

### Authoring Rules

- Write all summaries, rationales, comments, and diagnostics from scratch in your own words.
- Compatibility codes may be referenced as bare numbers or `SC1234`-style identifiers.
- Do not copy ShellCheck text or third-party snippets verbatim into committed code or comments.
