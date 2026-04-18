#!/bin/sh
build() {
	cd /tmp
	pwd
	cd ..
	cd /opt
	pwd
	cd - >/dev/null
}

checked_entry() {
	cd /tmp || return
	pwd
	cd ..
}

checked_restore() {
	cd /tmp
	pwd
	cd .. || return
}

wrapped_restore() {
	builtin cd /tmp
	pwd
	builtin cd ..
}
