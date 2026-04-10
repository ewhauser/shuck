#!/bin/sh
echo <<EOF
hello
EOF

echo hi <<-EOF
	content
	EOF

cat <<EOF
hello
EOF
