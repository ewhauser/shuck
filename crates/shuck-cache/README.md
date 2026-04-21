# shuck-cache

`shuck-cache` provides file-oriented cache keys and on-disk package caches used by `shuck` for
fast incremental runs.

The crate is generic enough to reuse in other tooling, but it is primarily designed around the
needs of the Shuck workspace and is still pre-1.0.
