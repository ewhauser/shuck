#!/bin/sh

# Invalid: negative values are not numeric exit statuses.
exit -1

# Invalid: words are not numeric exit statuses.
exit nope

# Valid: plain decimal status codes are okay.
exit 2
