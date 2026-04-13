#!/bin/sh

# Should trigger: stdout is redirected before the pipe.
cmd >/dev/null | next
cmd >out | next

# Should trigger: the middle segment is the one with the redirect.
left | mid >/dev/null | right

# Should not trigger: stderr-only redirect.
2>/dev/null | next

# Should not trigger: the redirect happens after the pipeline.
cmd | next >/dev/null

# Should not trigger: pipeall is a different operator.
cmd >/dev/null |& next
