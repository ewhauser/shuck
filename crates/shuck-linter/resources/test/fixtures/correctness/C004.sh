#!/bin/sh
cd /tmp

if cd /var; then
    pwd
fi

cd /opt || exit 1
