#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
default_corpus_dir="$repo_root/.cache/large-corpus"

usage() {
  echo "Usage: $0 [-l] [corpus-dir]"
  echo "  -l  List repos only (dry run)"
  echo "  corpus-dir defaults to $default_corpus_dir"
  exit 1
}

dry_run=false
while getopts "lh" opt; do
  case "$opt" in
    l) dry_run=true ;;
    h) usage ;;
    *) usage ;;
  esac
done
shift $((OPTIND - 1))

if [ "$#" -gt 1 ]; then
  usage
fi

corpus_dir=${1:-$default_corpus_dir}
scripts_dir="$corpus_dir/scripts"
clones_dir="$corpus_dir/clones"
manifest="$corpus_dir/manifest.yaml"

# Curated list of shell-heavy repositories.
# Explicitly excludes koalaman/shellcheck and related repos.
REPOS="
acmesh-official/acme.sh
ohmyzsh/ohmyzsh
nvm-sh/nvm
asdf-vm/asdf
pi-hole/pi-hole
dylanaraps/neofetch
rbenv/rbenv
pyenv/pyenv
rvm/rvm
bats-core/bats-core
zsh-users/zsh-autosuggestions
zsh-users/zsh-syntax-highlighting
romkatv/powerlevel10k
tj/n
jorgebucaran/fisher
dehydrated-io/dehydrated
oh-my-fish/oh-my-fish
megastep/makeself
sstephenson/bats
termux/termux-packages
void-linux/void-packages
google/oss-fuzz
bitnami/containers
community-scripts/ProxmoxVE
tteck/Proxmox
HariSekhon/DevOps-Bash-tools
docker-library/official-images
Bash-it/bash-it
sorin-ionescu/prezto
zsh-users/zsh-completions
zdharma-continuum/zinit
scop/bash-completion
dokku/dokku
docker-mailserver/docker-mailserver
docker/docker-bench-security
super-linter/super-linter
hwdsl2/setup-ipsec-vpn
Nyr/openvpn-install
angristan/openvpn-install
CISOfy/lynis
awslabs/git-secrets
openvpn/easy-rsa
sickcodes/Docker-OSX
quickemu-project/quickemu
89luca89/distrobox
dylanaraps/pure-bash-bible
dylanaraps/pure-sh-bible
tj/git-extras
alexanderepstein/Bash-Snippets
xwmx/nb
aristocratos/bashtop
pystardust/ani-cli
rupa/z
moovweb/gvm
nvie/gitflow
p8952/bocker
ko1nksm/shellspec
kward/shunit2
mathiasbynens/dotfiles
holman/dotfiles
thoughtbot/dotfiles
itzg/docker-minecraft-server
bin456789/reinstall
juewuy/ShellCrash
tmux-plugins/tpm
tmux-plugins/tmux-resurrect
nextcloud/docker
dockur/windows
basecamp/omarchy
LukeSmithxyz/LARBS
gentoo/gentoo
alpinelinux/aports
xwmx/nb
RetroPie/RetroPie-Setup
aristocratos/bashtop
SlackBuildsOrg/slackbuilds
GameServerManagers/LinuxGSM
v1s1t0r1sh3r3/airgeddon
CISOfy/lynis
leebaird/discover
233boy/v2ray
v2fly/fhs-install-v2ray
spiritLHLS/one-click-installation-script
lmc999/RegionRestrictionCheck
masonr/yet-another-bench-script
jessfraz/dotfiles
paulirish/dotfiles
fideloper/Vaprobash
swoodford/aws
bittorf/kalua
openrc/openrc
client9/shlib
helmuthdu/aui
"

# Detect shell from shebang line.
detect_shebang() {
  head -1 "$1" 2>/dev/null | grep -qE '^#!.*(sh|bash|dash|ksh|zsh)' && return 0
  return 1
}

# Sanitize a file path into a flat filename.
sanitize_path() {
  echo "$1" | sed 's|/|__|g'
}

if [ "$dry_run" = false ]; then
  mkdir -p "$scripts_dir" "$clones_dir"
  if [ ! -f "$manifest" ]; then
    printf 'download_date: %s\nrepos:\n' "$(date +%Y-%m-%d)" > "$manifest"
  fi
fi

for repo in $REPOS; do
  case "$repo" in
    *shellcheck*)
      echo "BLOCKED: $repo (shellcheck-related)"
      continue
      ;;
  esac

  owner=$(echo "$repo" | cut -d/ -f1)
  name=$(echo "$repo" | cut -d/ -f2)
  repo_key="${owner}__${name}"

  if [ "$dry_run" = false ] && grep -q "repo: $repo" "$manifest" 2>/dev/null; then
    echo "SKIP: $repo (already in manifest)"
    continue
  fi

  if [ "$dry_run" = true ]; then
    echo "WOULD CLONE: $repo"
    continue
  fi

  echo "==> Cloning $repo (shallow)..."
  clone_dest="$clones_dir/$repo_key"
  rm -rf "$clone_dest"

  if ! git clone --depth 1 --single-branch -q "https://github.com/$repo.git" "$clone_dest" 2>/dev/null; then
    echo "  FAILED to clone $repo, skipping"
    continue
  fi

  commit_sha=$(git -C "$clone_dest" rev-parse HEAD)

  find "$clone_dest" -type f \( -name '*.sh' -o -name '*.bash' -o -name '*.zsh' -o -name '*.ksh' \) | while read -r file; do
    rel_path=$(echo "$file" | sed "s|^$clone_dest/||")
    dest_name="${repo_key}__$(sanitize_path "$rel_path")"
    cp "$file" "$scripts_dir/$dest_name"
  done

  find "$clone_dest" -type f -not -name '*.sh' -not -name '*.bash' -not -name '*.zsh' -not -name '*.ksh' | while read -r file; do
    if detect_shebang "$file"; then
      rel_path=$(echo "$file" | sed "s|^$clone_dest/||")
      dest_name="${repo_key}__$(sanitize_path "$rel_path")"
      cp "$file" "$scripts_dir/$dest_name"
    fi
  done

  extracted=$(find "$scripts_dir" -name "${repo_key}__*" -type f | wc -l | tr -d ' ')
  echo "  Extracted $extracted scripts from $repo"

  cat >> "$manifest" <<EOF
  - repo: $repo
    commit: $commit_sha
    date: $(date +%Y-%m-%d)
    scripts_extracted: $extracted
EOF

  rm -rf "$clone_dest"
done

if [ "$dry_run" = false ]; then
  final_count=$(find "$scripts_dir" -type f | wc -l | tr -d ' ')
  echo ""
  echo "==> Done. $final_count total scripts in $scripts_dir"
fi
