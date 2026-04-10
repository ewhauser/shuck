#!/bin/sh
build() {
	cd /tmp
	pushd /var
	popd
	cd /opt || return
}

build
cd /root
