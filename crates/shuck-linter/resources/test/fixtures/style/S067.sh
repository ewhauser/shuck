#!/bin/sh

grep '*MAINTAINER' "$AUDIT_FILE"
grep "*ENTRYPOINT" "$AUDIT_FILE"
grep *CMD "$AUDIT_FILE"
grep -e '*LABEL' "$AUDIT_FILE"
grep --regexp '*EXPOSE' "$AUDIT_FILE"
grep -v '^*' "$AUDIT_FILE"

grep 'MAINTAINER*' "$AUDIT_FILE"
grep '.*MAINTAINER' "$AUDIT_FILE"
grep \*MAINTAINER "$AUDIT_FILE"
grep -F '*MAINTAINER' "$AUDIT_FILE"
grep -e'*MAINTAINER' "$AUDIT_FILE"
grep --regexp='*MAINTAINER' "$AUDIT_FILE"
grep '^*$' "$AUDIT_FILE"
grep '^*foo' "$AUDIT_FILE"
