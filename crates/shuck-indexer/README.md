# shuck-indexer

`shuck-indexer` builds positional and structural indexes over parsed shell scripts.

It sits between parsing and higher-level analysis by providing efficient lookups for lines,
comments, quoted regions, heredocs, command substitutions, and continuation lines. The crate is
published for downstream integrations but is still pre-1.0.
