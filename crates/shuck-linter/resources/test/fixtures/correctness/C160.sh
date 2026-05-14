#!/bin/bash

source ./lib.sh
. ../env.sh
source "lib/defaults.sh"

source "${BASH_SOURCE[0]%/*}/anchored.sh"
source ${BASH_SOURCE[0]%/*}"/split-anchored.sh"
source "$(dirname "$0")/also-anchored.sh"
source /etc/profile
source ~alice/profile
source "$(select_source_file)"
source "${BASH_SOURCE[0]%/*}suffix/not-anchored.sh"
source '${BASH_SOURCE[0]%/*}/single-quoted.sh'
# shellcheck source=./helper.sh
source '$(select_source_file)'
source ~${USER}/dynamic-home.sh
