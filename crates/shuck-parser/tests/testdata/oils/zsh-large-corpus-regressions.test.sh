## compare_shells: zsh bash mksh

# Extracted from the full large-corpus zsh parse harness on 2026-04-07.
# These snippets cover previously failing zsh parser surfaces in shuck.
# Every case in this fixture is now promoted to parse_ok in zsh mode.

#### ohmyzsh__ohmyzsh__lib__cli.zsh

# source: ohmyzsh__ohmyzsh__lib__cli.zsh
# surface: case pattern with literal prefix bare groups and sibling grouped arms

case "${words[2]}::${words[3]}" in
  plugin::(disable|enable|load))
    local -aU valid_plugins

    if [[ "${words[3]}" = disable ]]; then
      valid_plugins=($plugins)
    else
      valid_plugins=("$ZSH"/plugins/*/{_*,*.plugin.zsh}(-.N:h:t))
      [[ "${words[3]}" = enable ]] && valid_plugins=(${valid_plugins:|plugins})
    fi

    _describe 'plugin' valid_plugins ;;
  plugin::info)
    local -aU plugins
    plugins=("$ZSH"/plugins/*/{_*,*.plugin.zsh}(-.N:h:t))
    _describe 'plugin' plugins ;;
  theme::(set|use))
    local -aU themes
    themes=("$ZSH"/themes/*.zsh-theme(N:t:r))
    _describe 'theme' themes ;;
esac

#### ohmyzsh__ohmyzsh__lib__clipboard.zsh

# source: ohmyzsh__ohmyzsh__lib__clipboard.zsh
# surface: compact function body with background-pipe redirect inside an if ladder

  if [[ "${OSTYPE}" == darwin* ]] && (( ${+commands[pbcopy]} )) && (( ${+commands[pbpaste]} )); then
    function clipcopy() { cat "${1:-/dev/stdin}" | pbcopy; }
    function clippaste() { pbpaste; }
  elif [[ "${OSTYPE}" == (cygwin|msys)* ]]; then
    function clipcopy() { cat "${1:-/dev/stdin}" > /dev/clipboard; }
    function clippaste() { cat /dev/clipboard; }
  elif (( $+commands[clip.exe] )) && (( $+commands[powershell.exe] )); then
    function clipcopy() { cat "${1:-/dev/stdin}" | clip.exe; }
    function clippaste() { powershell.exe -noprofile -command Get-Clipboard; }
  elif [ -n "${WAYLAND_DISPLAY:-}" ] && (( ${+commands[wl-copy]} )) && (( ${+commands[wl-paste]} )); then
    function clipcopy() { cat "${1:-/dev/stdin}" | wl-copy &>/dev/null &|; }
    function clippaste() { wl-paste --no-newline; }
  elif [ -n "${DISPLAY:-}" ] && (( ${+commands[xsel]} )); then
    function clipcopy() { cat "${1:-/dev/stdin}" | xsel --clipboard --input; }
    function clippaste() { xsel --clipboard --output; }
  fi

#### ohmyzsh__ohmyzsh__lib__functions.zsh

# source: ohmyzsh__ohmyzsh__lib__functions.zsh
# regression surface: parse error at line 29, column 17: expected command

  # define the open command
  case "$OSTYPE" in
    darwin*)  open_cmd='open' ;;
    cygwin*)  open_cmd='cygstart' ;;
    linux*)   [[ "$(uname -r)" != *icrosoft* ]] && open_cmd='nohup xdg-open' || {
                open_cmd='cmd.exe /c start ""'
                [[ -e "$1" ]] && { 1="$(wslpath -w "${1:a}")" || return 1 }
                [[ "$1" = (http|https)://* ]] && {
                  1="$(echo "$1" | sed -E 's/([&|()<>^])/^\1/g')" || return 1
                }
              } ;;
    msys*)    open_cmd='start ""' ;;
    *)        echo "Platform $OSTYPE not supported"
              return 1
              ;;
  esac

#### ohmyzsh__ohmyzsh__lib__git.zsh

# source: ohmyzsh__ohmyzsh__lib__git.zsh
# surface: compact prompt helper functions after an or-list brace-group condition

# Use async version if setting is enabled, or unset but zsh version is at least 5.0.6.
# This avoids async prompt issues caused by previous zsh versions:
# - https://github.com/ohmyzsh/ohmyzsh/issues/12331
# - https://github.com/ohmyzsh/ohmyzsh/issues/12360
# TODO(2024-06-12): @mcornella remove workaround when CentOS 7 reaches EOL
local _style
if zstyle -t ':omz:alpha:lib:git' async-prompt \
  || { is-at-least 5.0.6 && zstyle -T ':omz:alpha:lib:git' async-prompt }; then
  function git_prompt_info() {
    if [[ -n "${_OMZ_ASYNC_OUTPUT[_omz_git_prompt_info]}" ]]; then
      echo -n "${_OMZ_ASYNC_OUTPUT[_omz_git_prompt_info]}"
    fi
  }

  function git_prompt_status() {
    if [[ -n "${_OMZ_ASYNC_OUTPUT[_omz_git_prompt_status]}" ]]; then
      echo -n "${_OMZ_ASYNC_OUTPUT[_omz_git_prompt_status]}"
    fi
  }
fi

#### ohmyzsh__ohmyzsh__lib__prompt_info_functions.zsh

# source: ohmyzsh__ohmyzsh__lib__prompt_info_functions.zsh
# surface: line-continued multi-name function header into a brace body

# Dummy implementations that return false to prevent command_not_found
# errors with themes, that implement these functions
# Real implementations will be used when the respective plugins are loaded
function chruby_prompt_info \
  rbenv_prompt_info \
  hg_prompt_info \
  pyenv_prompt_info \
{
  return 1
}

#### ohmyzsh__ohmyzsh__lib__termsupport.zsh

# source: ohmyzsh__ohmyzsh__lib__termsupport.zsh
# surface: jobspec case patterns with numeric ranges and mixed grouped alternatives

case "$jobspec" in
  <->) # %number argument:
    job_id=${jobspec} ;;
  ""|%|+) # empty, %% or %+ argument:
    job_id=${(k)jobstates[(r)*:+:*]} ;;
  -) # %- argument:
    job_id=${(k)jobstates[(r)*:-:*]} ;;
  [?]*) # %?string argument:
    job_id=${(k)jobtexts[(r)*${(Q)jobspec}*]} ;;
  *) # %string argument:
    job_id=${(k)jobtexts[(r)${(Q)jobspec}*]} ;;
esac

#### ohmyzsh__ohmyzsh__lib__theme-and-appearance.zsh

# source: ohmyzsh__ohmyzsh__lib__theme-and-appearance.zsh
# regression surface: parse error at line 63, column 20: expected command

# Find the option for using colors in ls, depending on the version
case "$OSTYPE" in
  netbsd*)
    # On NetBSD, test if `gls` (GNU ls) is installed (this one supports colors);
    # otherwise, leave ls as is, because NetBSD's ls doesn't support -G
    test-ls-args gls --color && alias ls='gls --color=tty'
    ;;
  openbsd*)
    # On OpenBSD, `gls` (ls from GNU coreutils) and `colorls` (ls from base,
    # with color and multibyte support) are available from ports.
    # `colorls` will be installed on purpose and can't be pulled in by installing
    # coreutils (which might be installed for ), so prefer it to `gls`.
    test-ls-args gls --color && alias ls='gls --color=tty'
    test-ls-args colorls -G && alias ls='colorls -G'
    ;;
  (darwin|freebsd)*)
    # This alias works by default just using $LSCOLORS
    test-ls-args ls -G && alias ls='ls -G'
    # Only use GNU ls if installed and there are user defaults for $LS_COLORS,
    # as the default coloring scheme is not very pretty
    zstyle -t ':omz:lib:theme-and-appearance' gnu-ls \
      && test-ls-args gls --color \
      && alias ls='gls --color=tty'
    ;;
  *)
    if test-ls-args ls --color; then
      alias ls='ls --color=tty'
    elif test-ls-args ls -G; then
      alias ls='ls -G'
    fi
    ;;
esac

#### ohmyzsh__ohmyzsh__plugins__autoenv__autoenv.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__autoenv__autoenv.plugin.zsh
# regression surface: parse error at line 25, column 11: expected 'do'

  # Locate autoenv installation
  if [[ -z $autoenv_dir ]]; then
    install_locations=(
      ~/.autoenv
      ~/.local/bin
      /usr/local/opt/autoenv
      /opt/homebrew/opt/autoenv
      /usr/local/bin
      /usr/share/autoenv-git
      ~/Library/Python/bin
      .venv/bin
      venv/bin
      env/bin
      .env/bin
    )
    for d ( $install_locations ); do
      if [[ -e $d/activate || -e $d/activate.sh ]]; then
        autoenv_dir=$d
        break
      fi
    done
  fi

#### ohmyzsh__ohmyzsh__plugins__battery__battery.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__battery__battery.plugin.zsh
# surface: empty compact helper function body between multiline helpers

function battery_pct_remaining() {
  if ! battery_is_charging; then
    battery_pct
  else
    echo "External Power"
  fi
}
function battery_time_remaining() { } # Not available on android
function battery_pct_prompt() {
  local battery_pct color
  battery_pct=$(battery_pct_remaining)
  if battery_is_charging; then
    echo "∞"
  else
    if [[ $battery_pct -gt 50 ]]; then
      color='green'
    elif [[ $battery_pct -gt 20 ]]; then
      color='yellow'
    else
      color='red'
    fi
    echo "%{$fg[$color]%}${battery_pct}%%%{$reset_color%}"
  fi
}

#### ohmyzsh__ohmyzsh__plugins__cabal__cabal.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__cabal__cabal.plugin.zsh
# regression surface: parse error at line 93, column 64: expected command

command -v cab >/dev/null 2>&1 && { compdef _cab_commands cab }

#### ohmyzsh__ohmyzsh__plugins__chruby__chruby.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__chruby__chruby.plugin.zsh
# regression surface: parse error at line 90, column 1: expected command

# Simple definition completer for ruby-build
if command ruby-build &> /dev/null; then
  _ruby-build() { compadd $(ruby-build --definitions) }
  compdef _ruby-build ruby-build
fi

#### ohmyzsh__ohmyzsh__plugins__cloudfoundry__cloudfoundry.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__cloudfoundry__cloudfoundry.plugin.zsh
# regression surface: parse error: expected compound command for function body

alias cfdm="cf domains"
alias cfsp="cf spaces"
function cfap() { cf app $1 }
function cfh.() { export CF_HOME=$PWD/.cf }
function cfh~() { export CF_HOME=~/.cf }
function cfhu() { unset CF_HOME }

#### ohmyzsh__ohmyzsh__plugins__colored-man-pages__colored-man-pages.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__colored-man-pages__colored-man-pages.plugin.zsh
# regression surface: parse error at line 32, column 9: expected 'do'

  # Convert associative array to plain array of NAME=VALUE items.
  local k v
  for k v in "${(@kv)less_termcap}"; do
    environment+=( "LESS_TERMCAP_${k}=${v}" )
  done

#### ohmyzsh__ohmyzsh__plugins__command-not-found__command-not-found.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__command-not-found__command-not-found.plugin.zsh
# regression surface: commented parenthesized zsh for header

for file (
  # Arch Linux. Must have pkgfile installed: https://wiki.archlinux.org/title/Zsh#pkgfile_"command_not_found"_handler
  /usr/share/doc/pkgfile/command-not-found.zsh
  # Void Linux: https://codeberg.org/classabbyamp/xbps-command-not-found
  /usr/share/zsh/plugins/xbps-command-not-found/xbps-command-not-found.zsh
); do
  if [[ -r "$file" ]]; then
    source "$file"
    break
  fi
done

#### ohmyzsh__ohmyzsh__plugins__dash__dash.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__dash__dash.plugin.zsh
# surface: parameter-flag array capture and match cleanup inside _dash()

_dash() {
  local -a enabled_docsets
  enabled_docsets=("${(@f)$(defaults read com.kapeli.dashdoc docsets | tr -d '\n' | grep -oE '\{.*?\}' | grep -E 'isEnabled = 1;')}")

  local docset keyword
  for docset in "$enabled_docsets[@]"; do
    keyword=''
    if [[ "$docset" =~ "keyword = ([^;]*);" ]]; then
      keyword="${match[1]//[\":]}"
    fi
    if [[ -n "$keyword" ]]; then
      docsets+=($keyword)
    fi
  done

  compadd -qS: -- "$docsets[@]"
}

#### ohmyzsh__ohmyzsh__plugins__debian__debian.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__debian__debian.plugin.zsh
# regression surface: parse error at line 162, column 72: expected 'do'

    for p in ${(f)"$(aptitude search -F "%p" --disable-columns \~i)"}; {
        cmd="${cmd} ${p}"
    }

#### ohmyzsh__ohmyzsh__plugins__extract__extract.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__extract__extract.plugin.zsh
# surface: compact brace groups after && and || inside case arms

case "${file:l}" in
  (*.tar.gz|*.tgz)
    (( $+commands[pigz] )) && { tar -I pigz -xvf "$full_path" } || tar zxvf "$full_path" ;;
  (*.tar.bz2|*.tbz|*.tbz2)
    (( $+commands[pbzip2] )) && { tar -I pbzip2 -xvf "$full_path" } || tar xvjf "$full_path" ;;
  (*.tar.xz|*.txz)
    (( $+commands[pixz] )) && { tar -I pixz -xvf "$full_path" } || {
      tar --xz --help &> /dev/null \
      && tar --xz -xvf "$full_path" \
      || xzcat "$full_path" | tar xvf -
    } ;;
esac

#### ohmyzsh__ohmyzsh__plugins__gcloud__gcloud.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__gcloud__gcloud.plugin.zsh
# regression surface: parse error at line 40, column 17: expected 'do'

  # Look for completion file in different paths
  for comp_file (
    "${CLOUDSDK_HOME}/completion.zsh.inc"             # default location
    "/usr/share/google-cloud-sdk/completion.zsh.inc"  # apt-based location
  ); do
    if [[ -f "${comp_file}" ]]; then
      source "${comp_file}"
      break
    fi
  done
  unset comp_file

#### ohmyzsh__ohmyzsh__plugins__genpass__genpass-apple

# source: ohmyzsh__ohmyzsh__plugins__genpass__genpass-apple
# regression surface: parse error at line 34, column 5: expected command

  # Sets REPLY to a uniformly distributed random number in [1, $1].
  # Requires: $1 <= 256.
  function -$0-rand() {
    local c
    while true; do
      sysread -s1 c || return
      # Avoid bias towards smaller numbers.
      (( #c < 256 / $1 * $1 )) && break
    done
    typeset -g REPLY=$((#c % $1 + 1))
  }

#### ohmyzsh__ohmyzsh__plugins__genpass__genpass-xkcd

# source: ohmyzsh__ohmyzsh__plugins__genpass__genpass-xkcd
# surface: nested repeat loops with zsh arithmetic char literals and redirect tail

{
  local c
  repeat ${1-1}; do
    print -rn -- $n
    repeat $n; do
      while true; do
        # Generate a random number in [0, 2**31).
        local -i rnd=0
        repeat 4; do
          sysread -s1 c || return
          (( rnd = (~(1 << 23) & rnd) << 8 | #c ))
        done
        # Avoid bias towards words in the beginning of the list.
        (( rnd < 16#7FFFFFFF / $#words * $#words )) || continue
        print -rn -- -$words[rnd%$#words+1]
        break
      done
    done
    print
  done
} </dev/urandom

#### ohmyzsh__ohmyzsh__plugins__git-extras__git-extras.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__git-extras__git-extras.plugin.zsh
# regression surface: parse error at line 418, column 70: expected command

# ------------------------------------------------------------------------------
# Description
# -----------
#
#  Completion script for git-extras (https://github.com/tj/git-extras).
#
#  This depends on and reuses some of the internals of the _git completion
#  function that ships with zsh itself. It will not work with the _git that ships
#  with git.
#
# ------------------------------------------------------------------------------
# Authors
# -------
#
#  * Alexis GRIMALDI (https://github.com/agrimaldi)
#  * spacewander (https://github.com/spacewander)
#
# ------------------------------------------------------------------------------
# Inspirations
# -----------
#
#  * git-extras (https://github.com/tj/git-extras)
#  * git-flow-completion (https://github.com/bobthecow/git-flow-completion)
#
# ------------------------------------------------------------------------------


# Internal functions
# These are a lot like their __git_* equivalents inside _git

__gitex_command_successful () {
  if (( ${#*:#0} > 0 )); then
    _message 'not a git repository'
    return 1
  fi
  return 0
}

__gitex_commits() {
    declare -A commits
    git log --oneline -15 | sed 's/\([[:alnum:]]\{7\}\) /\1:/' | while read commit
    do
        hash=$(echo $commit | cut -d':' -f1)
        commits[$hash]="$commit"
    done
    local ret=1
    _describe -t commits commit commits && ret=0
}

__gitex_remote_names() {
    local expl
    declare -a remote_names
    remote_names=(${(f)"$(_call_program remotes git remote 2>/dev/null)"})
    __gitex_command_successful || return
    _wanted remote-names expl remote-name compadd $* - $remote_names
}

__gitex_tag_names() {
    local expl
    declare -a tag_names
    tag_names=(${${(f)"$(_call_program tags git for-each-ref --format='"%(refname)"' refs/tags 2>/dev/null)"}#refs/tags/})
    __gitex_command_successful || return
    _wanted tag-names expl tag-name compadd $* - $tag_names
}


__gitex_branch_names() {
    local expl
    declare -a branch_names
    branch_names=(${${(f)"$(_call_program branchrefs git for-each-ref --format='"%(refname)"' refs/heads 2>/dev/null)"}#refs/heads/})
    __gitex_command_successful || return
    _wanted branch-names expl branch-name compadd $* - $branch_names
}

__gitex_specific_branch_names() {
    local expl
    declare -a branch_names
    branch_names=(${${(f)"$(_call_program branchrefs git for-each-ref --format='"%(refname)"' refs/heads/"$1" 2>/dev/null)"}#refs/heads/$1/})
    __gitex_command_successful || return
    _wanted branch-names expl branch-name compadd - $branch_names
}

__gitex_feature_branch_names() {
    __gitex_specific_branch_names 'feature'
}

__gitex_submodule_names() {
    local expl
    declare -a submodule_names
    submodule_names=(${(f)"$(_call_program branchrefs git submodule status | awk '{print $2}')"})  # '
    __gitex_command_successful || return
    _wanted submodule-names expl submodule-name compadd $* - $submodule_names
}


__gitex_author_names() {
    local expl
    declare -a author_names
    author_names=(${(f)"$(_call_program branchrefs git log --format='%aN' | sort -u)"})
    __gitex_command_successful || return
    _wanted author-names expl author-name compadd $* - $author_names
}

# subcommands
# new subcommand should be added in alphabetical order
_git-authors() {
    _arguments  -C \
        '(--list -l)'{--list,-l}'[show authors]' \
        '--no-email[without email]' \
}

_git-changelog() {
    _arguments \
        '(-l --list)'{-l,--list}'[list commits]' \
}

_git-clear() {
    _arguments \
        '(-f --force)'{-f,--force}'[force clear]' \
        '(-h --help)'{-h,--help}'[help message]' \
}

_git-coauthor() {
    _arguments \
        ':co-author[co-author to add]' \
        ':co-author-email[email address of co-author to add]'
}

_git-contrib() {
    _arguments \
        ':author:__gitex_author_names'
}


_git-count() {
    _arguments \
        '--all[detailed commit count]'
}

_git-create-branch() {
    local curcontext=$curcontext state line
    _arguments -C \
        ': :->command' \
        '*:: :->option-or-argument'

    case "$state" in
        (command)
            _arguments \
                '(--remote -r)'{--remote,-r}'[setup remote tracking branch]'
            ;;
        (option-or-argument)
            curcontext=${curcontext%:*}-$line[1]:
            case $line[1] in
                -r|--remote )
                    _arguments -C \
                        ':remote-name:__gitex_remote_names'
                    ;;
            esac
    esac
}

_git-delete-branch() {
    _arguments \
        ':branch-name:__gitex_branch_names'
}

_git-delete-squashed-branches() {
    _arguments \
        ':branch-name:__gitex_branch_names'
}


_git-delete-submodule() {
    _arguments \
        ':submodule-name:__gitex_submodule_names'
}


_git-delete-tag() {
    _arguments \
        ':tag-name:__gitex_tag_names'
}


_git-effort() {
    _arguments \
        '--above[ignore file with less than x commits]'
}


_git-extras() {
    local curcontext=$curcontext state line ret=1
    declare -A opt_args

    _arguments -C \
        ': :->command' \
        '*:: :->option-or-argument' && ret=0

    case $state in
        (command)
            declare -a commands
            commands=(
                'update:update git-extras'
            )
            _describe -t commands command commands && ret=0
            ;;
    esac

    _arguments \
        '(-v --version)'{-v,--version}'[show current version]'
}


_git-feature() {
    local curcontext=$curcontext state line ret=1
    declare -A opt_args

    _arguments -C \
        ': :->command' \
        '*:: :->option-or-argument' && ret=0

    case $state in
        (command)
            declare -a commands
            commands=(
                'finish:merge feature into the current branch'
            )
            _describe -t commands command commands && ret=0
            ;;
        (option-or-argument)
            curcontext=${curcontext%:*}-$line[1]:
            case $line[1] in
                (finish)
                    _arguments -C \
                        '--squash[Use squash merge]' \
                        ':branch-name:__gitex_feature_branch_names'
                    ;;
                -r|--remote )
                    _arguments -C \
                        ':remote-name:__gitex_remote_names'
                    ;;
            esac
            return 0
    esac

    _arguments \
        '(--remote -r)'{--remote,-r}'[setup remote tracking branch]'
}

_git-graft() {
    _arguments \
        ':src-branch-name:__gitex_branch_names' \
        ':dest-branch-name:__gitex_branch_names'
}

_git-guilt() {
    _arguments -C \
        '(--email -e)'{--email,-e}'[display author emails instead of names]' \
        '(--ignore-whitespace -w)'{--ignore-whitespace,-w}'[ignore whitespace only changes]' \
        '(--debug -d)'{--debug,-d}'[output debug information]' \
        '-h[output usage information]'
}

_git-ignore() {
    _arguments -C \
        '(--local -l)'{--local,-l}'[show local gitignore]' \
        '(--global -g)'{--global,-g}'[show global gitignore]' \
        '(--private -p)'{--private,-p}'[show repo gitignore]'
}


_git-info() {
    _arguments -C \
        '(--color -c)'{--color,-c}'[use color for information titles]' \
        '--no-config[do not show list all variables set in config file, along with their values]'
}


_git-merge-into() {
    _arguments '--ff-only[merge only fast-forward]'
    _arguments \
        ':src:__gitex_branch_names' \
        ':dest:__gitex_branch_names'
}

_git-missing() {
    _arguments \
        ':first-branch-name:__gitex_branch_names' \
        ':second-branch-name:__gitex_branch_names'
}

_git-release() {
    _arguments -C \
        '-c[Generates/populates the changelog with all commit message since the last tag.]' \
        '-r[The "remote" repository that is destination of a push operation.]' \
        '-m[use the custom commit information instead of the default message.]' \
        '-s[Create a signed and annotated tag.]' \
        '-u[Create a tag, annotated and signed with the given key.]' \
        '--semver[If the latest tag in your repo matches the semver format requirement, you could increase part of it as the new release tag.]' \
        '--prefix[Add a prefix string to semver to allow more complex tags.]' \
        '--no-empty-commit[Avoid creating empty commit if nothing could be committed.]' \
        '--[The arguments listed after "--" separator will be passed to pre/post-release hook.]'
}

_git-squash() {
    _arguments '--squash-msg[commit with the squashed commit messages]'
    _arguments \
        ':branch-name:__gitex_branch_names'
}

_git-stamp() {
    _arguments -C \
         '(--replace -r)'{--replace,-r}'[replace stamps with same id]'
}

_git-standup() {
    _arguments -C \
        '-a[Specify the author of commits. Use "all" to specify all authors.]' \
        '-d[Show history since N days ago]' \
        '-D[Specify the date format displayed in commit history]' \
        '-f[Fetch commits before showing history]' \
        '-g[Display GPG signed info]' \
        '-h[Display help message]' \
        '-L[Enable the inclusion of symbolic links]' \
        '-m[The depth of recursive directory search]' \
        '-B[Display the commits in branch groups]'
}

_git-summary() {
    _arguments '--line[summarize with lines rather than commits]'
    _arguments '--dedup-by-email[remove duplicate users by the email address]'
    _arguments '--no-merges[exclude merge commits]'
    __gitex_commits
}

_git-undo(){
    _arguments -C \
        '(--soft -s)'{--soft,-s}'[only rolls back the commit but changes remain un-staged]' \
        '(--hard -h)'{--hard,-h}'[wipes your commit(s)]'
}

zstyle -g existing_user_commands ':completion:*:*:git:*' user-commands

zstyle ':completion:*:*:git:*' user-commands $existing_user_commands \
    alias:'define, search and show aliases' \
    abort:'abort current revert, merge, rebase, or cherry-pick process' \
    archive-file:'export the current head of the git repository to an archive' \
    authors:'generate authors report' \
    browse:'open repo website in browser' \
    browse-ci:'open repo CI page in browser' \
    bug:'create bug branch' \
    bulk:'run bulk commands' \
    brv:'list branches sorted by their last commit date'\
    changelog:'generate a changelog report' \
    chore:'create chore branch' \
    clear-soft:'soft clean up a repository' \
    clear:'rigorously clean up a repository' \
    coauthor:'add a co-author to the last commit' \
    commits-since:'show commit logs since some date' \
    contrib:'show user contributions' \
    count:'show commit count' \
    create-branch:'create branches' \
    delete-branch:'delete branches' \
    delete-merged-branches:'delete merged branches' \
    delete-squashed-branches:'delete squashed branches' \
    delete-submodule:'delete submodules' \
    delete-tag:'delete tags' \
    delta:'lists changed files' \
    effort:'show effort statistics on file(s)' \
    extras:'awesome git utilities' \
    feature:'create/merge feature branch' \
    force-clone:'overwrite local repositories with clone' \
    fork:'fork a repo on GitHub' \
    fresh-branch:'create fresh branches' \
    gh-pages:'create the GitHub pages branch' \
    graft:'merge and destroy a given branch' \
    guilt:'calculate change between two revisions' \
    ignore-io:'get sample gitignore file' \
    ignore:'add .gitignore patterns' \
    info:'returns information on current repository' \
    local-commits:'list local commits' \
    lock:'lock a file excluded from version control' \
    locked:'ls files that have been locked' \
    magic:'commits everything with a generated message' \
    merge-into:'merge one branch into another' \
    merge-repo:'merge two repo histories' \
    missing:'show commits missing from another branch' \
    mr:'checks out a merge request locally' \
    obliterate:'rewrite past commits to remove some files' \
    paste:'send patches to pastebin sites' \
    pr:'checks out a pull request locally' \
    psykorebase:'rebase a branch with a merge commit' \
    pull-request:'create pull request to GitHub project' \
    reauthor:'replace the author and/or committer identities in commits and tags' \
    rebase-patch:'rebases a patch' \
    refactor:'create refactor branch' \
    release:'commit, tag and push changes to the repository' \
    rename-branch:'rename a branch' \
    rename-tag:'rename a tag' \
    rename-remote:'rename a remote' \
    repl:'git read-eval-print-loop' \
    reset-file:'reset one file' \
    root:'show path of root' \
    scp:'copy files to ssh compatible `git-remote`' \
    sed:'replace patterns in git-controlled files' \
    setup:'set up a git repository' \
    show-merged-branches:'show merged branches' \
    show-tree:'show branch tree of commit history' \
    show-unmerged-branches:'show unmerged branches' \
    squash:'import changes from a branch' \
    stamp:'stamp the last commit message' \
    standup:'recall the commit history' \
    summary:'show repository summary' \
    sync:'sync local branch with remote branch' \
    touch:'touch and add file to the index' \
    undo:'remove latest commits' \
    unlock:'unlock a file excluded from version control' \
    utimes:'change files modification time to their last commit date'

#### ohmyzsh__ohmyzsh__plugins__git__git.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__git__git.plugin.zsh
# regression surface: multi-target parenthesized zsh for header

# Logic for adding warnings on deprecated aliases or functions
local old_name new_name
for old_name new_name (
  current_branch  git_current_branch
); do
  aliases[$old_name]="deprecated: ${old_name} -> ${new_name}
    $new_name"
done

#### ohmyzsh__ohmyzsh__plugins__globalias__globalias.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__globalias__globalias.plugin.zsh
# regression surface: parse error at line 6, column 35: expected ']]' to close conditional expression

globalias() {
   # Get last word to the left of the cursor:
   # (z) splits into words using shell parsing
   # (A) makes it an array even if there's only one element
   local word=${${(Az)LBUFFER}[-1]}
   if [[ $GLOBALIAS_FILTER_VALUES[(Ie)$word] -eq 0 ]]; then
      zle _expand_alias
      zle expand-word
   fi
   zle self-insert
}
zle -N globalias

#### ohmyzsh__ohmyzsh__plugins__keychain__keychain.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__keychain__keychain.plugin.zsh
# surface: nameless function keyword command with a multiline body

function {
	local agents
	local -a identities
	return 0
}

#### ohmyzsh__ohmyzsh__plugins__macos__music

# source: ohmyzsh__ohmyzsh__plugins__macos__music
# surface: multi-name function keyword header with trailing parens

function music itunes() {
  local APP_NAME=Music sw_vers=$(sw_vers -productVersion 2>/dev/null)
  print -- "$APP_NAME $sw_vers"
}

#### ohmyzsh__ohmyzsh__plugins__pj__pj.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__pj__pj.plugin.zsh
# regression surface: parse error at line 15, column 15: expected 'do'

  for basedir ($PROJECT_PATHS); do
    if [[ -d "$basedir/$project" ]]; then
      $cmd "$basedir/$project"
      return
    fi
  done

#### ohmyzsh__ohmyzsh__plugins__rake-fast__rake-fast.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__rake-fast__rake-fast.plugin.zsh
# surface: compact trailing or-list brace group in helper predicate

_rake_does_task_list_need_generating () {
  _rake_tasks_missing || _rake_tasks_version_changed || _rakefile_has_changes || { _is_rails_app && _tasks_changed }
}

#### ohmyzsh__ohmyzsh__plugins__rbenv__rbenv.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__rbenv__rbenv.plugin.zsh
# surface: compact helper definitions clustered in an else branch

if [[ $FOUND_RBENV -eq 1 ]]; then
  function rbenv_prompt_info() {
    local ruby=${$(current_ruby):gs/%/%%} gemset=${$(current_gemset):gs/%/%%}
    echo -n "${ZSH_THEME_RUBY_PROMPT_PREFIX}"
    [[ -n "$gemset" ]] && echo -n "${ruby}@${gemset}" || echo -n "${ruby}"
    echo "${ZSH_THEME_RUBY_PROMPT_SUFFIX}"
  }
else
  alias rubies="ruby -v"
  function gemsets() { echo "not supported" }
  function current_ruby() { echo "not supported" }
  function current_gemset() { echo "not supported" }
  function gems() { echo "not supported" }
  function rbenv_prompt_info() {
    echo -n "${ZSH_THEME_RUBY_PROMPT_PREFIX}"
    echo -n "system: $(ruby -v | cut -f-2 -d ' ' | sed 's/%/%%/g')"
    echo "${ZSH_THEME_RUBY_PROMPT_SUFFIX}"
  }
fi

#### ohmyzsh__ohmyzsh__plugins__scd__scd

# source: ohmyzsh__ohmyzsh__plugins__scd__scd
# regression surface: parse error at line 102, column 19: expected command

# load scd-ignore patterns if available
if [[ -s $SCD_IGNORE ]]; then
    setopt noglob
    <$SCD_IGNORE \
    while read p; do
        [[ $p != [\#]* ]] || continue
        [[ -n $p ]] || continue
        # expand leading tilde if it has valid expansion
        if [[ $p == [~]* ]] && ( : ${~p} ) 2>/dev/null; then
            p=${~p}
        fi
        scdignore[$p]=1
    done
    setopt glob
fi

#### ohmyzsh__ohmyzsh__plugins__scd__scd.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__scd__scd.plugin.zsh
# regression surface: parse error at line 11, column 1: expected command

# extracted ## If the scd function exists, define a change-directory-hook function
# extracted ## to record visited directories in the scd index.
if [[ ${+functions[scd]} == 1 ]]; then
    chpwd_scd() { scd --add $PWD }
    autoload -Uz add-zsh-hook
    add-zsh-hook chpwd chpwd_scd
fi

#### ohmyzsh__ohmyzsh__plugins__screen__screen.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__screen__screen.plugin.zsh
# regression surface: parse error: expected compound command for function body

  # Unset title() function defined in lib/termsupport.zsh to prevent
  # overwriting our screen titles
  title(){}

#### ohmyzsh__ohmyzsh__plugins__shrink-path__shrink-path.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__shrink-path__shrink-path.plugin.zsh
# regression surface: parse error at line 151, column 46: expected 'do'

        if (( named )) {
                for part in ${(k)nameddirs}; {
                        [[ $dir == ${nameddirs[$part]}(/*|) ]] && dir=${dir/#${nameddirs[$part]}/\~$part}
                }
        }
        (( tilde )) && dir=${dir/#$HOME/\~}

#### ohmyzsh__ohmyzsh__plugins__sublime-merge__sublime-merge.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__sublime-merge__sublime-merge.plugin.zsh
# surface: top-level anonymous paren function with nested compact helpers

() {
	local _sublime_linux_paths
	_sublime_linux_paths=("$HOME/bin/sublime_merge")
	for _sublime_merge_path in $_sublime_linux_paths; do
		if [[ -a $_sublime_merge_path ]]; then
			sm_run() { $_sublime_merge_path "$@" >/dev/null 2>&1 &| }
			ssm_run_sudo() {sudo $_sublime_merge_path "$@" >/dev/null 2>&1}
			alias ssm=ssm_run_sudo
			alias sm=sm_run
			break
		fi
	done
}

#### ohmyzsh__ohmyzsh__plugins__term_tab__term_tab.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__term_tab__term_tab.plugin.zsh
# regression surface: parse error at line 30, column 56: expected command

  case $OSTYPE in
    solaris*) dirs=( ${(M)${${(f)"$(pgrep -U $UID -x zsh|xargs pwdx)"}:#$$:*}%%/*} ) ;;
    linux*) dirs=( /proc/${^$(pidof zsh):#$$}/cwd(N:A) ) ;;
    darwin*) dirs=( $( lsof -d cwd -c zsh -a -w -Fn | sed -n 's/^n//p' ) ) ;;
  esac
  dirs=( ${(D)dirs} )

#### ohmyzsh__ohmyzsh__plugins__urltools__urltools.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__urltools__urltools.plugin.zsh
# surface: compact helper definitions in an elif ladder

if [[ $(whence python3) != "" && "x$URLTOOLS_METHOD" = "xpython" ]]; then
    alias urlencode='python3 encode'
    alias urldecode='python3 decode'
elif [[ $(whence xxd) != "" && ( "x$URLTOOLS_METHOD" = "x" || "x$URLTOOLS_METHOD" = "xshell" ) ]]; then
    function urlencode() {echo $@ | tr -d "\n" | xxd -plain | sed "s/\(..\)/%\1/g"}
    function urldecode() {printf $(echo -n $@ | sed 's/\\/\\\\/g;s/\(%\)\([0-9a-fA-F][0-9a-fA-F]\)/\\x\2/g')"\n"}
elif [[ $(whence ruby) != "" && ( "x$URLTOOLS_METHOD" = "x" || "x$URLTOOLS_METHOD" = "xruby" ) ]]; then
    alias urlencode='ruby encode'
    alias urldecode='ruby decode'
fi

#### ohmyzsh__ohmyzsh__plugins__virtualenvwrapper__virtualenvwrapper.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__virtualenvwrapper__virtualenvwrapper.plugin.zsh
# regression surface: parse error at line 1, column 10: expected function name

function {
    # search in these locations for the init script:
    for virtualenvwrapper in $commands[virtualenvwrapper_lazy.sh] \
      $commands[virtualenvwrapper.sh] \
      /usr/share/virtualenvwrapper/virtualenvwrapper{_lazy,}.sh \
      /usr/local/bin/virtualenvwrapper{_lazy,}.sh \
      /usr/bin/virtualenvwrapper{_lazy,}.sh \
      /etc/bash_completion.d/virtualenvwrapper \
      /usr/share/bash-completion/completions/virtualenvwrapper \
      $HOME/.local/bin/virtualenvwrapper.sh
    do
        if [[ -f "$virtualenvwrapper" ]]; then
            source "$virtualenvwrapper"
            return
        fi
    done
    print "[oh-my-zsh] virtualenvwrapper plugin: Cannot find virtualenvwrapper.sh.\n"\
          "Please install with \`pip install virtualenvwrapper\`" >&2
    return 1
}

#### ohmyzsh__ohmyzsh__plugins__wd__wd.sh

# source: ohmyzsh__ohmyzsh__plugins__wd__wd.sh
# surface: regex elif ladder with arithmetic subscript conditional

    if [[ $point =~ "^[\.]+$" ]]
    then
        wd_exit_fail "Warp point cannot be just dots"
    elif [[ $point =~ "[[:space:]]+" ]]
    then
        wd_exit_fail "Warp point should not contain whitespace"
    elif [[ $point =~ : ]] || [[ $point =~ / ]]
    then
        wd_exit_fail "Warp point contains illegal character (:/)"
    elif (($cmdnames[(Ie)$point]))
    then
        wd_exit_fail "Warp point name cannot be a wd command (see wd -h for a full list)"
    elif [[ ${points[$point]} == "" ]] || [ ! -z "$force" ]
    then
        wd_remove "$point" > /dev/null
        printf "%q:%s\n" "${point}" "${PWD/#$HOME/~}" >> "$wd_config_file"
        if (whence sort >/dev/null); then
            local config_tmp=$(mktemp "${TMPDIR:-/tmp}/wd.XXXXXXXXXX")
            # use 'cat' below to ensure we respect $wd_config_file as a symlink
            command sort -o "${config_tmp}" "$wd_config_file" && command cat "${config_tmp}" >| "$wd_config_file" && command rm "${config_tmp}"
        fi
    else
        wd_exit_warn "Warp point '${point}' already exists. Use 'add --force' to overwrite."
    fi

#### ohmyzsh__ohmyzsh__plugins__xcode__xcode.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__xcode__xcode.plugin.zsh
# regression surface: parse error at line 140, column 15: expected 'do'

# Print the active version, using xcselv's notion of versions
function _omz_xcode_print_active_version {
  emulate -L zsh
  local -A xcode_versions
  local versions version active_path
  _omz_xcode_locate_versions
  active_path=$(xcode-select -p)
  active_path=${active_path%%/Contents/Developer*}
  versions=(${(kni)xcode_versions})
  for version ($versions); do
    if [[ "${xcode_versions[$version]}" == $active_path ]]; then
      printf "%s (%s)\n" $version $active_path
      return
    fi
  done
  printf "%s (%s)\n" "<unknown>" $active_path
}

#### ohmyzsh__ohmyzsh__plugins__z__z.plugin.zsh

# source: ohmyzsh__ohmyzsh__plugins__z__z.plugin.zsh
# regression surface: parse error at line 1022, column 38: expected 'done'

# extracted ################################################################################
# Zsh-z - jump around with Zsh - A native Zsh version of z without awk, sort,
# date, or sed
#
# https://github.com/agkozak/zsh-z
#
# Copyright (c) 2018-2025 Alexandros Kozak
#
# Permission is hereby granted, free of charge, to any person obtaining a copy
# of this software and associated documentation files (the "Software"), to deal
# in the Software without restriction, including without limitation the rights
# to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
# copies of the Software, and to permit persons to whom the Software is
# furnished to do so, subject to the following conditions:
#
# The above copyright notice and this permission notice shall be included in all
# copies or substantial portions of the Software.
#
# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
# IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
# FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
# AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
# LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
# OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
# SOFTWARE.
#
# z (https://github.com/rupa/z) is copyright (c) 2009 rupa deadwyler and
# licensed under the WTFPL license, Version 2.
#
# Zsh-z maintains a jump-list of the directories you actually use.
#
# INSTALL:
#   * put something like this in your .zshrc:
#       source /path/to/zsh-z.plugin.zsh
#   * cd around for a while to build up the database
#
# USAGE:
#   * z foo       cd to the most frecent directory matching foo
#   * z foo bar   cd to the most frecent directory matching both foo and bar
#                   (e.g. /foo/bat/bar/quux)
#   * z -r foo    cd to the highest ranked directory matching foo
#   * z -t foo    cd to most recently accessed directory matching foo
#   * z -l foo    List matches instead of changing directories
#   * z -e foo    Echo the best match without changing directories
#   * z -c foo    Restrict matches to subdirectories of PWD
#   * z -x        Remove a directory (default: PWD) from the database
#   * z -xR       Remove a directory (default: PWD) and its subdirectories from
#                   the database
#
# ENVIRONMENT VARIABLES:
#
#   ZSHZ_CASE -> if `ignore', pattern matching is case-insensitive; if `smart',
#     pattern matching is case-insensitive only when the pattern is all
#     lowercase
#   ZSHZ_CD -> the directory-changing command that is used (default: builtin cd)
#   ZSHZ_CMD -> name of command (default: z)
#   ZSHZ_COMPLETION -> completion method (default: 'frecent'; 'legacy' for
#     alphabetic sorting)
#   ZSHZ_DATA -> name of datafile (default: ~/.z)
#   ZSHZ_EXCLUDE_DIRS -> array of directories to exclude from your database
#     (default: empty)
#   ZSHZ_KEEP_DIRS -> array of directories that should not be removed from the
#     database, even if they are not currently available (default: empty)
#   ZSHZ_MAX_SCORE -> maximum combined score the database entries can have
#     before beginning to age (default: 9000)
#   ZSHZ_NO_RESOLVE_SYMLINKS -> '1' prevents symlink resolution
#   ZSHZ_OWNER -> your username (if you want use Zsh-z while using sudo -s)
#   ZSHZ_UNCOMMON -> if 1, do not jump to "common directories," but rather drop
#     subdirectories based on what the search string was (default: 0)
# extracted ################################################################################

autoload -U is-at-least

if ! is-at-least 4.3.11; then
  print "Zsh-z requires Zsh v4.3.11 or higher." >&2 && exit
fi

# extracted ############################################################
# The help message
#
# Globals:
#   ZSHZ_CMD
# extracted ############################################################
_zshz_usage() {
  print "Usage: ${ZSHZ_CMD:-${_Z_CMD:-z}} [OPTION]... [ARGUMENT]
Jump to a directory that you have visited frequently or recently, or a bit of both, based on the partial string ARGUMENT.

With no ARGUMENT, list the directory history in ascending rank.

  --add Add a directory to the database
  -c    Only match subdirectories of the current directory
  -e    Echo the best match without going to it
  -h    Display this help and exit
  -l    List all matches without going to them
  -r    Match by rank
  -t    Match by recent access
  -x    Remove a directory from the database (by default, the current directory)
  -xR   Remove a directory and its subdirectories from the database (by default, the current directory)" |
    fold -s -w $COLUMNS >&2
}

# Load zsh/datetime module, if necessary
(( ${+EPOCHSECONDS} )) || zmodload zsh/datetime

# Global associative array for internal use
typeset -gA ZSHZ

# Fallback utilities in case Zsh lacks zsh/files (as is the case with MobaXterm)
ZSHZ[CHOWN]='chown'
ZSHZ[MV]='mv'
ZSHZ[RM]='rm'
# Try to load zsh/files utilities
if [[ ${builtins[zf_chown]-} != 'defined' ||
      ${builtins[zf_mv]-}    != 'defined' ||
      ${builtins[zf_rm]-}    != 'defined' ]]; then
  zmodload -F zsh/files b:zf_chown b:zf_mv b:zf_rm &> /dev/null
fi
# Use zsh/files, if it is available
[[ ${builtins[zf_chown]-} == 'defined' ]] && ZSHZ[CHOWN]='zf_chown'
[[ ${builtins[zf_mv]-} == 'defined' ]] && ZSHZ[MV]='zf_mv'
[[ ${builtins[zf_rm]-} == 'defined' ]] && ZSHZ[RM]='zf_rm'

# Load zsh/system, if necessary
[[ ${modules[zsh/system]-} == 'loaded' ]] || zmodload zsh/system &> /dev/null

# Make sure ZSHZ_EXCLUDE_DIRS has been declared so that other scripts can
# simply append to it
(( ${+ZSHZ_EXCLUDE_DIRS} )) || typeset -gUa ZSHZ_EXCLUDE_DIRS

# Determine if zsystem flock is available
zsystem supports flock &> /dev/null && ZSHZ[USE_FLOCK]=1

# Determine if `print -v' is supported
is-at-least 5.3.0 && ZSHZ[PRINTV]=1

# extracted ############################################################
# The Zsh-z Command
#
# Globals:
#   ZSHZ
#   ZSHZ_CASE
#   ZSHZ_CD
#   ZSHZ_COMPLETION
#   ZSHZ_DATA
#   ZSHZ_DEBUG
#   ZSHZ_EXCLUDE_DIRS
#   ZSHZ_KEEP_DIRS
#   ZSHZ_MAX_SCORE
#   ZSHZ_OWNER
#
# Arguments:
#   $* Command options and arguments
# extracted ############################################################
zshz() {

  # Don't use `emulate -L zsh' - it breaks PUSHD_IGNORE_DUPS
  setopt LOCAL_OPTIONS NO_KSH_ARRAYS NO_SH_WORD_SPLIT EXTENDED_GLOB UNSET
  (( ZSHZ_DEBUG )) && setopt LOCAL_OPTIONS WARN_CREATE_GLOBAL

  local REPLY
  local -a lines

  # Allow the user to specify a custom datafile in $ZSHZ_DATA (or legacy $_Z_DATA)
  local custom_datafile="${ZSHZ_DATA:-$_Z_DATA}"

  # If a datafile was provided as a standalone file without a directory path
  # print a warning and exit
  if [[ -n ${custom_datafile} && ${custom_datafile} != */* ]]; then
    print "ERROR: You configured a custom Zsh-z datafile (${custom_datafile}), but have not specified its directory." >&2
    exit
  fi

  # If the user specified a datafile, use that or default to ~/.z
  # If the datafile is a symlink, it gets dereferenced
  local datafile=${${custom_datafile:-$HOME/.z}:A}

  # If the datafile is a directory, print a warning and exit
  if [[ -d $datafile ]]; then
    print "ERROR: Zsh-z's datafile (${datafile}) is a directory." >&2
    exit
  fi

  # Make sure that the datafile exists before attempting to read it or lock it
  # for writing
  [[ -f $datafile ]] || { mkdir -p "${datafile:h}" && touch "$datafile" }

  # Bail if we don't own the datafile and $ZSHZ_OWNER is not set
  [[ -z ${ZSHZ_OWNER:-${_Z_OWNER}} && -f $datafile && ! -O $datafile ]] &&
    return

  # Load the datafile into an array and parse it
  lines=( ${(f)"$(< $datafile)"} )
  # Discard entries that are incomplete or incorrectly formatted
  lines=( ${(M)lines:#/*\|[[:digit:]]##[.,]#[[:digit:]]#\|[[:digit:]]##} )

  ############################################################
  # Add a path to or remove one from the datafile
  #
  # Globals:
  #   ZSHZ
  #   ZSHZ_EXCLUDE_DIRS
  #   ZSHZ_OWNER
  #
  # Arguments:
  #   $1 Which action to perform (--add/--remove)
  #   $2 The path to add
  ############################################################
  _zshz_add_or_remove_path() {
    local action=${1}
    shift

    if [[ $action == '--add' ]]; then

      # TODO: The following tasks are now handled by _agkozak_precmd. Dead code?

      # Don't add $HOME
      [[ $* == $HOME ]] && return

      # Don't track directory trees excluded in ZSHZ_EXCLUDE_DIRS
      local exclude
      for exclude in ${(@)ZSHZ_EXCLUDE_DIRS:-${(@)_Z_EXCLUDE_DIRS}}; do
        case $* in
          ${exclude}|${exclude}/*) return ;;
        esac
      done
    fi

    # A temporary file that gets copied over the datafile if all goes well
    local tempfile="${datafile}.${RANDOM}"

    # See https://github.com/rupa/z/pull/199/commits/ed6eeed9b70d27c1582e3dd050e72ebfe246341c
    if (( ZSHZ[USE_FLOCK] )); then

      local lockfd

      # Grab exclusive lock (released when function exits)
      zsystem flock -f lockfd "$datafile" 2> /dev/null || return

    fi

    integer tmpfd
    case $action in
      --add)
        exec {tmpfd}>|"$tempfile"  # Open up tempfile for writing
        _zshz_update_datafile $tmpfd "$*"
        local ret=$?
        ;;
      --remove)
        local xdir  # Directory to be removed

        if (( ${ZSHZ_NO_RESOLVE_SYMLINKS:-${_Z_NO_RESOLVE_SYMLINKS}} )); then
          [[ -d ${${*:-${PWD}}:a} ]] && xdir=${${*:-${PWD}}:a}
        else
          [[ -d ${${*:-${PWD}}:A} ]] && xdir=${${*:-${PWD}}:a}
        fi

        local -a lines_to_keep
        if (( ${+opts[-R]} )); then
          # Prompt user before deleting entire database
          if [[ $xdir == '/' ]] && ! read -q "?Delete entire Zsh-z database? "; then
            print && return 1
          fi
          # All of the lines that don't match the directory to be deleted
          lines_to_keep=( ${lines:#${xdir}\|*} )
          # Or its subdirectories
          lines_to_keep=( ${lines_to_keep:#${xdir%/}/**} )
        else
          # All of the lines that don't match the directory to be deleted
          lines_to_keep=( ${lines:#${xdir}\|*} )
        fi
        if [[ $lines != "$lines_to_keep" ]]; then
          lines=( $lines_to_keep )
        else
          return 1  # The $PWD isn't in the datafile
        fi
        exec {tmpfd}>|"$tempfile"  # Open up tempfile for writing
        print -u $tmpfd -l -- $lines
        local ret=$?
        ;;
    esac

    if (( tmpfd != 0 )); then
      # Close tempfile
      exec {tmpfd}>&-
    fi

    if (( ret != 0 )); then
      # Avoid clobbering the datafile if the write to tempfile failed
      ${ZSHZ[RM]} -f "$tempfile"
      return $ret
    fi

    local owner
    owner=${ZSHZ_OWNER:-${_Z_OWNER}}

    if (( ZSHZ[USE_FLOCK] )); then
      # An unsual case: if inside Docker container where datafile could be bind
      # mounted
      if [[ -r '/proc/1/cgroup' && "$(< '/proc/1/cgroup')" == *docker* ]]; then
        print "$(< "$tempfile")" > "$datafile" 2> /dev/null
        ${ZSHZ[RM]} -f "$tempfile"
      # All other cases
      else
        ${ZSHZ[MV]} "$tempfile" "$datafile" 2> /dev/null ||
            ${ZSHZ[RM]} -f "$tempfile"
      fi

      if [[ -n $owner ]]; then
        ${ZSHZ[CHOWN]} ${owner}:"$(id -ng ${owner})" "$datafile"
      fi
    else
      if [[ -n $owner ]]; then
        ${ZSHZ[CHOWN]} "${owner}":"$(id -ng "${owner}")" "$tempfile"
      fi
      ${ZSHZ[MV]} -f "$tempfile" "$datafile" 2> /dev/null ||
          ${ZSHZ[RM]} -f "$tempfile"
    fi

    # In order to make z -x work, we have to disable zsh-z's adding
    # to the database until the user changes directory and the
    # chpwd_functions are run
    if [[ $action == '--remove' ]]; then
      ZSHZ[DIRECTORY_REMOVED]=1
    fi
  }

  ############################################################
  # Read the current datafile contents, update them, "age" them
  # when the total rank gets high enough, and print the new
  # contents to STDOUT.
  #
  # Globals:
  #   ZSHZ_KEEP_DIRS
  #   ZSHZ_MAX_SCORE
  #
  # Arguments:
  #   $1 File descriptor linked to tempfile
  #   $2 Path to be added to datafile
  ############################################################
  _zshz_update_datafile() {

    integer fd=$1
    local -A rank time

    # Characters special to the shell (such as '[]') are quoted with backslashes
    # See https://github.com/rupa/z/issues/246
    local add_path=${(q)2}

    local -a existing_paths
    local now=$EPOCHSECONDS line dir
    local path_field rank_field time_field count x

    rank[$add_path]=1
    time[$add_path]=$now

    # Remove paths from database if they no longer exist
    for line in $lines; do
      if [[ ! -d ${line%%\|*} ]]; then
        for dir in ${(@)ZSHZ_KEEP_DIRS}; do
          if [[ ${line%%\|*} == ${dir}/* ||
                ${line%%\|*} == $dir     ||
                $dir == '/' ]]; then
            existing_paths+=( $line )
          fi
        done
      else
        existing_paths+=( $line )
      fi
    done
    lines=( $existing_paths )

    for line in $lines; do
      path_field=${(q)line%%\|*}
      rank_field=${${line%\|*}#*\|}
      time_field=${line##*\|}

      # When a rank drops below 1, drop the path from the database
      (( rank_field < 1 )) && continue

      if [[ $path_field == $add_path ]]; then
        rank[$path_field]=$rank_field
        (( rank[$path_field]++ ))
        time[$path_field]=$now
      else
        rank[$path_field]=$rank_field
        time[$path_field]=$time_field
      fi
      (( count += rank_field ))
    done
    if (( count > ${ZSHZ_MAX_SCORE:-${_Z_MAX_SCORE:-9000}} )); then
      # Aging
      for x in ${(k)rank}; do
        print -u $fd -- "$x|$(( 0.99 * rank[$x] ))|${time[$x]}" || return 1
      done
    else
      for x in ${(k)rank}; do
        print -u $fd -- "$x|${rank[$x]}|${time[$x]}" || return 1
      done
    fi
  }

  ############################################################
  # The original tab completion method
  #
  # String processing is smartcase -- case-insensitive if the
  # search string is lowercase, case-sensitive if there are
  # any uppercase letters. Spaces in the search string are
  # treated as *'s in globbing. Read the contents of the
  # datafile and print matches to STDOUT.
  #
  # Arguments:
  #   $1 The string to be completed
  ############################################################
  _zshz_legacy_complete() {

    local line path_field path_field_normalized

    # Replace spaces in the search string with asterisks for globbing
    1=${1//[[:space:]]/*}

    for line in $lines; do

      path_field=${line%%\|*}

      path_field_normalized=$path_field
      if (( ZSHZ_TRAILING_SLASH )); then
        path_field_normalized=${path_field%/}/
      fi

      # If the search string is all lowercase, the search will be case-insensitive
      if [[ $1 == "${1:l}" && ${path_field_normalized:l} == *${~1}* ]]; then
        print -- $path_field
      # Otherwise, case-sensitive
      elif [[ $path_field_normalized == *${~1}* ]]; then
        print -- $path_field
      fi

    done
    # TODO: Search strings with spaces in them are currently treated case-
    # insensitively.
  }

  ############################################################
  # `print' or `printf' to REPLY
  #
  # Variable assignment through command substitution, of the
  # form
  #
  #   foo=$( bar )
  #
  # requires forking a subshell; on Cygwin/MSYS2/WSL1 that can
  # be surprisingly slow. Zsh-z avoids doing that by printing
  # values to the variable REPLY. Since Zsh v5.3.0 that has
  # been possible with `print -v'; for earlier versions of the
  # shell, the values are placed on the editing buffer stack
  # and then `read' into REPLY.
  #
  # Globals:
  #   ZSHZ
  #
  # Arguments:
  #   Options and parameters for `print'
  ############################################################
  _zshz_printv() {
    # NOTE: For a long time, ZSH's `print -v' had a tendency
    # to mangle multibyte strings:
    #
    #   https://www.zsh.org/mla/workers/2020/msg00307.html
    #
    # The bug was fixed in late 2020:
    #
    #   https://github.com/zsh-users/zsh/commit/b6ba74cd4eaec2b6cb515748cf1b74a19133d4a4#diff-32bbef18e126b837c87b06f11bfc61fafdaa0ed99fcb009ec53f4767e246b129
    #
    # In order to support shells with the bug, we must use a form of `printf`,
    # which does not exhibit the undesired behavior. See
    #
    #   https://www.zsh.org/mla/workers/2020/msg00308.html

    if (( ZSHZ[PRINTV] )); then
      builtin print -v REPLY -f %s $@
    else
      builtin print -z $@
      builtin read -rz REPLY
    fi
  }

  ############################################################
  # If matches share a common root, find it, and put it in
  # REPLY for _zshz_output to use.
  #
  # Arguments:
  #   $1 Name of associative array of matches and ranks
  ############################################################
  _zshz_find_common_root() {
    local -a common_matches
    local x short

    common_matches=( ${(@Pk)1} )

    for x in ${(@)common_matches}; do
      if [[ -z $short ]] || (( $#x < $#short )) || [[ $x != ${short}/* ]]; then
        short=$x
      fi
    done

    [[ $short == '/' ]] && return

    for x in ${(@)common_matches}; do
      [[ $x != $short* ]] && return
    done

    _zshz_printv -- $short
  }

  ############################################################
  # Calculate a common root, if there is one. Then do one of
  # the following:
  #
  #   1) Print a list of completions in frecent order;
  #   2) List them (z -l) to STDOUT; or
  #   3) Put a common root or best match into REPLY
  #
  # Globals:
  #   ZSHZ_UNCOMMON
  #
  # Arguments:
  #   $1 Name of an associative array of matches and ranks
  #   $2 The best match or best case-insensitive match
  #   $3 Whether to produce a completion, a list, or a root or
  #        match
  ############################################################
  _zshz_output() {

    local match_array=$1 match=$2 format=$3
    local common k x
    local -a descending_list output
    local -A output_matches

    output_matches=( ${(Pkv)match_array} )

    _zshz_find_common_root $match_array
    common=$REPLY

    case $format in

      completion)
        for k in ${(@k)output_matches}; do
          _zshz_printv -f "%.2f|%s" ${output_matches[$k]} $k
          descending_list+=( ${(f)REPLY} )
          REPLY=''
        done
        descending_list=( ${${(@On)descending_list}#*\|} )
        print -l $descending_list
        ;;

      list)
        local path_to_display
        for x in ${(k)output_matches}; do
          if (( ${output_matches[$x]} )); then
            path_to_display=$x
            (( ZSHZ_TILDE )) &&
              path_to_display=${path_to_display/#${HOME}/\~}
            _zshz_printv -f "%-10d %s\n" ${output_matches[$x]} $path_to_display
            output+=( ${(f)REPLY} )
            REPLY=''
          fi
        done
        if [[ -n $common ]]; then
          (( ZSHZ_TILDE )) && common=${common/#${HOME}/\~}
          (( $#output > 1 )) && printf "%-10s %s\n" 'common:' $common
        fi
        # -lt
        if (( $+opts[-t] )); then
          for x in ${(@On)output}; do
            print -- $x
          done
        # -lr
        elif (( $+opts[-r] )); then
          for x in ${(@on)output}; do
            print -- $x
          done
        # -l
        else
          for x in ${(@on)output}; do
            print $x
          done
        fi
        ;;

      *)
        if (( ! ZSHZ_UNCOMMON )) && [[ -n $common ]]; then
          _zshz_printv -- $common
        else
          _zshz_printv -- ${(P)match}
        fi
        ;;
    esac
  }

  ############################################################
  # Match a pattern by rank, time, or a combination of the
  # two, and output the results as completions, a list, or a
  # best match.
  #
  # Globals:
  #   ZSHZ
  #   ZSHZ_CASE
  #   ZSHZ_KEEP_DIRS
  #   ZSHZ_OWNER
  #
  # Arguments:
  #   #1 Pattern to match
  #   $2 Matching method (rank, time, or [default] frecency)
  #   $3 Output format (completion, list, or [default] store
  #     in REPLY
  ############################################################
  _zshz_find_matches() {
    setopt LOCAL_OPTIONS NO_EXTENDED_GLOB

    local fnd=$1 method=$2 format=$3

    local -a existing_paths
    local line dir path_field rank_field time_field rank dx escaped_path_field
    local -A matches imatches
    local best_match ibest_match hi_rank=-9999999999 ihi_rank=-9999999999

    # Remove paths from database if they no longer exist
    for line in $lines; do
      if [[ ! -d ${line%%\|*} ]]; then
        for dir in ${(@)ZSHZ_KEEP_DIRS}; do
          if [[ ${line%%\|*} == ${dir}/* ||
                ${line%%\|*} == $dir     ||
                $dir == '/' ]]; then
            existing_paths+=( $line )
          fi
        done
      else
        existing_paths+=( $line )
      fi
    done
    lines=( $existing_paths )

    for line in $lines; do
      path_field=${line%%\|*}
      rank_field=${${line%\|*}#*\|}
      time_field=${line##*\|}

      case $method in
        rank) rank=$rank_field ;;
        time) (( rank = time_field - EPOCHSECONDS )) ;;
        *)
          # Frecency routine
          (( dx = EPOCHSECONDS - time_field ))
          rank=$(( 10000 * rank_field * (3.75/( (0.0001 * dx + 1) + 0.25)) ))
          ;;
      esac

      # Use spaces as wildcards
      local q=${fnd//[[:space:]]/\*}

      # If $ZSHZ_TRAILING_SLASH is set, use path_field with a trailing slash for matching.
      local path_field_normalized=$path_field
      if (( ZSHZ_TRAILING_SLASH )); then
        path_field_normalized=${path_field%/}/
      fi

      # If $ZSHZ_CASE is 'ignore', be case-insensitive.
      #
      # If it's 'smart', be case-insensitive unless the string to be matched
      # includes capital letters.
      #
      # Otherwise, the default behavior of Zsh-z is to match case-sensitively if
      # possible, then to fall back on a case-insensitive match if possible.
      if [[ $ZSHZ_CASE == 'smart' && ${1:l} == $1 &&
            ${path_field_normalized:l} == ${~q:l} ]]; then
        imatches[$path_field]=$rank
      elif [[ $ZSHZ_CASE != 'ignore' && $path_field_normalized == ${~q} ]]; then
        matches[$path_field]=$rank
      elif [[ $ZSHZ_CASE != 'smart' && ${path_field_normalized:l} == ${~q:l} ]]; then
        imatches[$path_field]=$rank
      fi

      # Escape characters that would cause "invalid subscript" errors
      # when accessing the associative array.
      escaped_path_field=${path_field//'\'/'\\'}
      escaped_path_field=${escaped_path_field//'`'/'\`'}
      escaped_path_field=${escaped_path_field//'('/'\('}
      escaped_path_field=${escaped_path_field//')'/'\)'}
      escaped_path_field=${escaped_path_field//'['/'\['}
      escaped_path_field=${escaped_path_field//']'/'\]'}

      if (( matches[$escaped_path_field] )) &&
         (( matches[$escaped_path_field] > hi_rank )); then
        best_match=$path_field
        hi_rank=${matches[$escaped_path_field]}
      elif (( imatches[$escaped_path_field] )) &&
           (( imatches[$escaped_path_field] > ihi_rank )); then
        ibest_match=$path_field
        ihi_rank=${imatches[$escaped_path_field]}
        ZSHZ[CASE_INSENSITIVE]=1
      fi
    done

    # Return 1 when there are no matches
    [[ -z $best_match && -z $ibest_match ]] && return 1

    if [[ -n $best_match ]]; then
      _zshz_output matches best_match $format
    elif [[ -n $ibest_match ]]; then
      _zshz_output imatches ibest_match $format
    fi
  }

  # THE MAIN ROUTINE

  local -A opts

  zparseopts -E -D -A opts -- \
    -add \
    -complete \
    c \
    e \
    h \
    -help \
    l \
    r \
    R \
    t \
    x

  if [[ $1 == '--' ]]; then
    shift
  elif [[ -n ${(M)@:#-*} && -z $compstate ]]; then
    print "Improper option(s) given."
    _zshz_usage
    return 1
  fi

  local opt output_format method='frecency' fnd prefix req

  for opt in ${(k)opts}; do
    case $opt in
      --add)
        [[ ! -d $* ]] && return 1
        local dir
        # Cygwin and MSYS2 have a hard time with relative paths expressed from /
        if [[ $OSTYPE == (cygwin|msys) && $PWD == '/' && $* != /* ]]; then
          set -- "/$*"
        fi
        if (( ${ZSHZ_NO_RESOLVE_SYMLINKS:-${_Z_NO_RESOLVE_SYMLINKS}} )); then
          dir=${*:a}
        else
          dir=${*:A}
        fi
        _zshz_add_or_remove_path --add "$dir"
        return
        ;;
      --complete)
        if [[ -s $datafile && ${ZSHZ_COMPLETION:-frecent} == 'legacy' ]]; then
          _zshz_legacy_complete "$1"
          return
        fi
        output_format='completion'
        ;;
      -c) [[ $* == ${PWD}/* || $PWD == '/' ]] || prefix="$PWD " ;;
      -h|--help)
        _zshz_usage
        return
        ;;
      -l) output_format='list' ;;
      -r) method='rank' ;;
      -t) method='time' ;;
      -x)
        # Cygwin and MSYS2 have a hard time with relative paths expressed from /
        if [[ $OSTYPE == (cygwin|msys) && $PWD == '/' && $* != /* ]]; then
          set -- "/$*"
        fi
        _zshz_add_or_remove_path --remove $*
        return
        ;;
    esac
  done
  req="$*"
  fnd="$prefix$*"

  [[ -n $fnd && $fnd != "$PWD " ]] || {
    [[ $output_format != 'completion' ]] && output_format='list'
  }

  #########################################################
  # Allow the user to specify directory-changing command
  # using $ZSHZ_CD (default: builtin cd).
  #
  # Globals:
  #   ZSHZ_CD
  #
  # Arguments:
  #   $* Path
  #########################################################
  zshz_cd() {
    setopt LOCAL_OPTIONS NO_WARN_CREATE_GLOBAL

    if [[ -z $ZSHZ_CD ]]; then
      builtin cd "$*"
    else
      ${=ZSHZ_CD} "$*"
    fi
  }

  #########################################################
  # If $ZSHZ_ECHO == 1, display paths as you jump to them.
  # If it is also the case that $ZSHZ_TILDE == 1, display
  # the home directory as a tilde.
  #########################################################
  _zshz_echo() {
    if (( ZSHZ_ECHO )); then
      if (( ZSHZ_TILDE )); then
        print ${PWD/#${HOME}/\~}
      else
        print $PWD
      fi
    fi
  }

  if [[ ${@: -1} == /* ]] && (( ! $+opts[-e] && ! $+opts[-l] )); then
    # cd if possible; echo the new path if $ZSHZ_ECHO == 1
    [[ -d ${@: -1} ]] && zshz_cd ${@: -1} && _zshz_echo && return
  fi

  # With option -c, make sure query string matches beginning of matches;
  # otherwise look for matches anywhere in paths

  # zpm-zsh/colors has a global $c, so we'll avoid math expressions here
  if [[ ! -z ${(tP)opts[-c]} ]]; then
    _zshz_find_matches "$fnd*" $method $output_format
  else
    _zshz_find_matches "*$fnd*" $method $output_format
  fi

  local ret2=$?

  local cd
  cd=$REPLY

  # New experimental "uncommon" behavior
  #
  # If the best choice at this point is something like /foo/bar/foo/bar, and the  # search pattern is `bar', go to /foo/bar/foo/bar; but if the search pattern
  # is `foo', go to /foo/bar/foo
  if (( ZSHZ_UNCOMMON )) && [[ -n $cd ]]; then
    if [[ -n $cd ]]; then

      # In the search pattern, replace spaces with *
      local q=${fnd//[[:space:]]/\*}
      q=${q%/} # Trailing slash has to be removed

      # As long as the best match is not case-insensitive
      if (( ! ZSHZ[CASE_INSENSITIVE] )); then
        # Count the number of characters in $cd that $q matches
        local q_chars=$(( ${#cd} - ${#${cd//${~q}/}} ))
        # Try dropping directory elements from the right; stop when it affects
        # how many times the search pattern appears
        until (( ( ${#cd:h} - ${#${${cd:h}//${~q}/}} ) != q_chars )); do
          cd=${cd:h}
        done

      # If the best match is case-insensitive
      else
        local q_chars=$(( ${#cd} - ${#${${cd:l}//${~${q:l}}/}} ))
        until (( ( ${#cd:h} - ${#${${${cd:h}:l}//${~${q:l}}/}} ) != q_chars )); do
          cd=${cd:h}
        done
      fi

      ZSHZ[CASE_INSENSITIVE]=0
    fi
  fi

  if (( ret2 == 0 )) && [[ -n $cd ]]; then
    if (( $+opts[-e] )); then               # echo
      (( ZSHZ_TILDE )) && cd=${cd/#${HOME}/\~}
      print -- "$cd"
    else
      # cd if possible; echo the new path if $ZSHZ_ECHO == 1
      [[ -d $cd ]] && zshz_cd "$cd" && _zshz_echo
    fi
  else
    # if $req is a valid path, cd to it; echo the new path if $ZSHZ_ECHO == 1
    if ! (( $+opts[-e] || $+opts[-l] )) && [[ -d $req ]]; then
      zshz_cd "$req" && _zshz_echo
    else
      return $ret2
    fi
  fi
}

alias ${ZSHZ_CMD:-${_Z_CMD:-z}}='zshz 2>&1'

# extracted ############################################################
# precmd - add path to datafile unless `z -x' has just been
#   run
#
# Globals:
#   ZSHZ
# extracted ############################################################
_zshz_precmd() {
  # Protect against `setopt NO_UNSET'
  setopt LOCAL_OPTIONS UNSET

  # Do not add PWD to datafile when in HOME directory, or
  # if `z -x' has just been run
  [[ $PWD == "$HOME" ]] || (( ZSHZ[DIRECTORY_REMOVED] )) && return

  # Don't track directory trees excluded in ZSHZ_EXCLUDE_DIRS
  local exclude
  for exclude in ${(@)ZSHZ_EXCLUDE_DIRS:-${(@)_Z_EXCLUDE_DIRS}}; do
    case $PWD in
      ${exclude}|${exclude}/*) return ;;
    esac
  done

  # It appears that forking a subshell is so slow in Windows that it is better
  # just to add the PWD to the datafile in the foreground
  if [[ $OSTYPE == (cygwin|msys) ]]; then
      zshz --add "$PWD"
  else
      (zshz --add "$PWD" &)
  fi

  # See https://github.com/rupa/z/pull/247/commits/081406117ea42ccb8d159f7630cfc7658db054b6
  : $RANDOM
}

# extracted ############################################################
# chpwd
#
# When the $PWD is removed from the datafile with `z -x',
# Zsh-z refrains from adding it again until the user has
# left the directory.
#
# Globals:
#   ZSHZ
# extracted ############################################################
_zshz_chpwd() {
  ZSHZ[DIRECTORY_REMOVED]=0
}

autoload -Uz add-zsh-hook

add-zsh-hook precmd _zshz_precmd
add-zsh-hook chpwd _zshz_chpwd

# extracted ############################################################
# Completion
# extracted ############################################################

# Standardized $0 handling
# https://zdharma-continuum.github.io/Zsh-100-Commits-Club/Zsh-Plugin-Standard.html
0="${${ZERO:-${0:#$ZSH_ARGZERO}}:-${(%):-%N}}"
0="${${(M)0:#/*}:-$PWD/$0}"

(( ${fpath[(ie)${0:A:h}]} <= ${#fpath} )) || fpath=( "${0:A:h}" "${fpath[@]}" )

# extracted ############################################################
# zsh-z functions
# extracted ############################################################
ZSHZ[FUNCTIONS]='_zshz_usage
                 _zshz_add_or_remove_path
                 _zshz_update_datafile
                 _zshz_legacy_complete
                 _zshz_printv
                 _zshz_find_common_root
                 _zshz_output
                 _zshz_find_matches
                 zshz
                 _zshz_precmd
                 _zshz_chpwd
                 _zshz'

# extracted ############################################################
# Enable WARN_NESTED_VAR for functions listed in
#   ZSHZ[FUNCTIONS]
# extracted ############################################################
(( ${+ZSHZ_DEBUG} )) && () {
  if is-at-least 5.4.0; then
    local x
    for x in ${=ZSHZ[FUNCTIONS]}; do
      functions -W $x
    done
  fi
}

# extracted ############################################################
# Unload function
#
# See https://github.com/agkozak/Zsh-100-Commits-Club/blob/master/Zsh-Plugin-Standard.adoc#unload-fun
#
# Globals:
#   ZSHZ
#   ZSHZ_CMD
# extracted ############################################################
zsh-z_plugin_unload() {
  emulate -L zsh

  add-zsh-hook -D precmd _zshz_precmd
  add-zsh-hook -d chpwd _zshz_chpwd

  local x
  for x in ${=ZSHZ[FUNCTIONS]}; do
    (( ${+functions[$x]} )) && unfunction $x
  done

  unset ZSHZ

  fpath=( "${(@)fpath:#${0:A:h}}" )

  (( ${+aliases[${ZSHZ_CMD:-${_Z_CMD:-z}}]} )) &&
    unalias ${ZSHZ_CMD:-${_Z_CMD:-z}}

  unfunction $0
}

# vim: fdm=indent:ts=2:et:sts=2:sw=2:

#### ohmyzsh__ohmyzsh__tools__changelog.sh

# source: ohmyzsh__ohmyzsh__tools__changelog.sh
# regression surface: parse error at line 267, column 12: expected 'do'

  # Remove commits that were reverted
  local hash rhash
  for hash rhash in ${(kv)reverts}; do
    if (( ${+types[$rhash]} )); then
      # Remove revert commit
      unset "types[$hash]" "subjects[$hash]" "scopes[$hash]" "breaking[$hash]"
      # Remove reverted commit
      unset "types[$rhash]" "subjects[$rhash]" "scopes[$rhash]" "breaking[$rhash]"
    fi
  done

#### ohmyzsh__ohmyzsh__tools__check_for_upgrade.sh

# source: ohmyzsh__ohmyzsh__tools__check_for_upgrade.sh
# regression surface: parse error at line 221, column 108: expected command

    # If in reminder mode or user has typed input, show reminder and exit
    if [[ "$update_mode" = reminder ]] || { [[ "$update_mode" != background-alpha ]] && has_typed_input }; then
      printf '\r\e[0K' # move cursor to first column and clear whole line
      echo "[oh-my-zsh] It's time to update! You can do that by running \`omz update\`"
      return 0
    fi

#### minimization__zsh_for_paren_do_done

for version ($versions); do
  print -r -- "$version"
done

#### minimization__zsh_for_paren_brace

for version ($versions); {
  print -r -- "$version"
}

#### minimization__zsh_for_multi_target_in

for key value in a b c d; do
  print -r -- "$key:$value"
done

#### minimization__zsh_for_digit_targets

for 1 2 3; do
  print -r -- "$1|$2|$3"
done

#### minimization__zsh_case_suffix_group

case "$mode" in
  plugin::(disable|enable|load))
    print -r -- "$mode"
    ;;
esac

#### minimization__zsh_case_numeric_range

case "$jobspec" in
  <->)
    print -r -- "$jobspec"
    ;;
esac

#### minimization__zsh_case_wrapper_alternatives

case $line in
  (#* | <->..<->)
    print -nP %F{blue}
    ;;
esac

#### minimization__zsh_case_group_suffix

case "$OSTYPE" in
  (darwin|freebsd)*)
    print -r -- "$OSTYPE"
    ;;
esac

#### minimization__zsh_case_semipipe

case $2 in
  cygwin_nt-10.0-i686)   bin='cygwin32/bin'  ;|
  cygwin_nt-10.0-x86_64) bin='cygwin64/bin'  ;|
  *)                     print -r -- "${bin:-fallback}" ;;
esac

#### minimization__zsh_conditional_subscript_unary

[[ -z $opts[(r)-P] ]]

#### minimization__zsh_conditional_subscript_arith

[[ $GLOBALIAS_FILTER_VALUES[(Ie)$word] -eq 0 ]]

#### minimization__zsh_conditional_pattern_backrefs

[[ "$buf" == (#b)(*)(${~pat})* ]]

#### minimization__zsh_conditional_pattern_anchors

[[ $buffer != (#s)[$'\t -~']#(#e) ]]

#### minimization__zsh_conditional_pattern_bare_group

[[ $OPTARG != (|+|-)<->(|.<->)(|[eE](|-|+)<->) ]]

#### minimization__zsh_arithmetic_subscript_ref

(( $+aliases[(e)$1] ))
(( $cmdnames[(Ie)$point] ))

#### minimization__zsh_arithmetic_char_literal

(( #c < 256 / $1 * $1 ))
(( rnd = (~(1 << 23) & rnd) << 8 | #c ))

#### minimization__zsh_parameter_modifier_groups

print ${(Az)LBUFFER} ${(s./.)_p9k__cwd} ${(pj./.)parts[1,MATCH]}

#### minimization__zsh_parameter_word_target

print ${^$(pidof zsh):#$$}

#### ohmyzsh__ohmyzsh__tools__upgrade.sh

# source: ohmyzsh__ohmyzsh__tools__upgrade.sh
# surface: url case patterns with optional trailing suffix groups

# Update upstream remote to ohmyzsh org
git remote -v | while read remote url extra; do
  case "$url" in
  git://github.com/robbyrussell/oh-my-zsh(|.git))
    # Update out-of-date "unauthenticated git protocol on port 9418" to https
    git remote set-url "$remote" "https://github.com/ohmyzsh/ohmyzsh.git" ;;
  https://github.com/robbyrussell/oh-my-zsh(|.git))
    git remote set-url "$remote" "https://github.com/ohmyzsh/ohmyzsh.git" ;;
  git@github.com:robbyrussell/oh-my-zsh(|.git))
    git remote set-url "$remote" "git@github.com:ohmyzsh/ohmyzsh.git" ;;
  https://github.com/ohmyzsh/ohmyzsh(|.git)) ;;
  git@github.com:ohmyzsh/ohmyzsh(|.git)) ;;
  *) continue ;;
  esac
  git config --local oh-my-zsh.remote "$remote"
  break
done

#### romkatv__powerlevel10k__config__p10k-classic.zsh

# source: romkatv__powerlevel10k__config__p10k-classic.zsh
# regression surface: parse error at line 442, column 5: syntax error: empty elif clause

    if (( VCS_STATUS_COMMITS_AHEAD || VCS_STATUS_COMMITS_BEHIND )); then
      # ⇣42 if behind the remote.
      (( VCS_STATUS_COMMITS_BEHIND )) && res+=" ${clean}⇣${VCS_STATUS_COMMITS_BEHIND}"
      # ⇡42 if ahead of the remote; no leading space if also behind the remote: ⇣42⇡42.
      (( VCS_STATUS_COMMITS_AHEAD && !VCS_STATUS_COMMITS_BEHIND )) && res+=" "
      (( VCS_STATUS_COMMITS_AHEAD  )) && res+="${clean}⇡${VCS_STATUS_COMMITS_AHEAD}"
    elif [[ -n $VCS_STATUS_REMOTE_BRANCH ]]; then
      # Tip: Uncomment the next line to display '=' if up to date with the remote.
      # res+=" ${clean}="
    fi

#### romkatv__powerlevel10k__config__p10k-lean-8colors.zsh

# source: romkatv__powerlevel10k__config__p10k-lean-8colors.zsh
# regression surface: parse error at line 433, column 5: syntax error: empty elif clause

    if (( VCS_STATUS_COMMITS_AHEAD || VCS_STATUS_COMMITS_BEHIND )); then
      # ⇣42 if behind the remote.
      (( VCS_STATUS_COMMITS_BEHIND )) && res+=" ${clean}⇣${VCS_STATUS_COMMITS_BEHIND}"
      # ⇡42 if ahead of the remote; no leading space if also behind the remote: ⇣42⇡42.
      (( VCS_STATUS_COMMITS_AHEAD && !VCS_STATUS_COMMITS_BEHIND )) && res+=" "
      (( VCS_STATUS_COMMITS_AHEAD  )) && res+="${clean}⇡${VCS_STATUS_COMMITS_AHEAD}"
    elif [[ -n $VCS_STATUS_REMOTE_BRANCH ]]; then
      # Tip: Uncomment the next line to display '=' if up to date with the remote.
      # res+=" ${clean}="
    fi

#### romkatv__powerlevel10k__config__p10k-lean.zsh

# source: romkatv__powerlevel10k__config__p10k-lean.zsh
# regression surface: parse error at line 433, column 5: syntax error: empty elif clause

    if (( VCS_STATUS_COMMITS_AHEAD || VCS_STATUS_COMMITS_BEHIND )); then
      # ⇣42 if behind the remote.
      (( VCS_STATUS_COMMITS_BEHIND )) && res+=" ${clean}⇣${VCS_STATUS_COMMITS_BEHIND}"
      # ⇡42 if ahead of the remote; no leading space if also behind the remote: ⇣42⇡42.
      (( VCS_STATUS_COMMITS_AHEAD && !VCS_STATUS_COMMITS_BEHIND )) && res+=" "
      (( VCS_STATUS_COMMITS_AHEAD  )) && res+="${clean}⇡${VCS_STATUS_COMMITS_AHEAD}"
    elif [[ -n $VCS_STATUS_REMOTE_BRANCH ]]; then
      # Tip: Uncomment the next line to display '=' if up to date with the remote.
      # res+=" ${clean}="
    fi

#### romkatv__powerlevel10k__config__p10k-rainbow.zsh

# source: romkatv__powerlevel10k__config__p10k-rainbow.zsh
# regression surface: parse error at line 443, column 5: syntax error: empty elif clause

    if (( VCS_STATUS_COMMITS_AHEAD || VCS_STATUS_COMMITS_BEHIND )); then
      # ⇣42 if behind the remote.
      (( VCS_STATUS_COMMITS_BEHIND )) && res+=" ${clean}⇣${VCS_STATUS_COMMITS_BEHIND}"
      # ⇡42 if ahead of the remote; no leading space if also behind the remote: ⇣42⇡42.
      (( VCS_STATUS_COMMITS_AHEAD && !VCS_STATUS_COMMITS_BEHIND )) && res+=" "
      (( VCS_STATUS_COMMITS_AHEAD  )) && res+="${clean}⇡${VCS_STATUS_COMMITS_AHEAD}"
    elif [[ -n $VCS_STATUS_REMOTE_BRANCH ]]; then
      # Tip: Uncomment the next line to display '=' if up to date with the remote.
      # res+=" ${clean}="
    fi

#### romkatv__powerlevel10k__gitstatus__gitstatus.plugin.zsh

# source: romkatv__powerlevel10k__gitstatus__gitstatus.plugin.zsh
# regression surface: parse error at line 162, column 41: expected ']]' to close conditional expression

  local opt dir callback OPTARG
  local -i no_diff OPTIND
  local -F timeout=-1
  while getopts ":d:c:t:p" opt; do
    case $opt in
      +p) no_diff=0;;
      p)  no_diff=1;;
      d)  dir=$OPTARG;;
      c)  callback=$OPTARG;;
      t)
        if [[ $OPTARG != (|+|-)<->(|.<->)(|[eE](|-|+)<->) ]]; then
          print -ru2 -- "gitstatus_query: invalid -t argument: $OPTARG"
          return 1
        fi
        timeout=OPTARG
      ;;
      \?) print -ru2 -- "gitstatus_query: invalid option: $OPTARG"           ; return 1;;
      :)  print -ru2 -- "gitstatus_query: missing required argument: $OPTARG"; return 1;;
      *)  print -ru2 -- "gitstatus_query: invalid option: $opt"              ; return 1;;
    esac
  done

#### romkatv__powerlevel10k__gitstatus__mbuild

# source: romkatv__powerlevel10k__gitstatus__mbuild
# surface: repeated ;| case terminators with a multiline fallthrough arm

local tmp env bin intro flags=(-w)
case $2 in
  cygwin_nt-10.0-i686)   bin='cygwin32/bin'  ;|
  cygwin_nt-10.0-x86_64) bin='cygwin64/bin'  ;|
  msys_nt-10.0-i686)     bin='msys32/usr/bin';|
  msys_nt-10.0-x86_64)   bin='msys64/usr/bin';|
  cygwin_nt-10.0-*)
    tmp='/cygdrive/c/tmp'
  ;|
  msys_nt-10.0-*)
    tmp='/c/tmp'
    env='MSYSTEM=MSYS'
    intro+='PATH="$PATH:/usr/bin/site_perl:/usr/bin/vendor_perl:/usr/bin/core_perl"'
    ;;
esac

#### romkatv__powerlevel10k__internal__configure.zsh

# source: romkatv__powerlevel10k__internal__configure.zsh
# regression surface: parse error at line 84, column 2: expected command

# Fewer than 47 columns will probably work. Haven't tried it.
typeset -gr __p9k_wizard_columns=47
# The bottleneck is ask_tails with nerd fonts. Everything else works fine with 12 lines.
typeset -gr __p9k_wizard_lines=14
typeset -gr __p9k_zd=${ZDOTDIR:-$HOME}
typeset -gr __p9k_zd_u=${${${(q)__p9k_zd}/#(#b)${(q)HOME}(|\/*)/'~'$match[1]}//\%/%%}
typeset -gr __p9k_zshrc=${${:-$__p9k_zd/.zshrc}:A}
typeset -gr __p9k_zshrc_u=$__p9k_zd_u/.zshrc
typeset -gr __p9k_root_dir_u=${${${(q)__p9k_root_dir}/#(#b)${(q)HOME}(|\/*)/'~'$match[1]}//\%/%%}

function _p9k_can_configure() {
  [[ $1 == '-q' ]] && local -i q=1 || local -i q=0
  function $0_error() {
    (( q )) || print -rP "%1F[ERROR]%f %Bp10k configure%b: $1" >&2
  }
  typeset -g __p9k_cfg_path_o=${POWERLEVEL9K_CONFIG_FILE:=${ZDOTDIR:-~}/.p10k.zsh}
  typeset -g __p9k_cfg_basename=${__p9k_cfg_path_o:t}
  typeset -g __p9k_cfg_path=${__p9k_cfg_path_o:A}
  typeset -g __p9k_cfg_path_u=${${${(q)__p9k_cfg_path_o}/#(#b)${(q)HOME}(|\/*)/'~'$match[1]}//\%/%%}
  {
    [[ -e $__p9k_zd ]]         || { $0_error "$__p9k_zd_u does not exist";       return 1 }
    [[ -d $__p9k_zd ]]         || { $0_error "$__p9k_zd_u is not a directory";   return 1 }
    [[ ! -d $__p9k_cfg_path ]] || { $0_error "$__p9k_cfg_path_u is a directory"; return 1 }
    [[ ! -d $__p9k_zshrc ]]    || { $0_error "$__p9k_zshrc_u is a directory";    return 1 }

    local dir=${__p9k_cfg_path:h}
    while [[ ! -e $dir && $dir != ${dir:h} ]]; do dir=${dir:h}; done
    if [[ ! -d $dir ]]; then
      $0_error "cannot create $__p9k_cfg_path_u because ${dir//\%/%%} is not a directory"
      return 1
    fi
    if [[ ! -w $dir ]]; then
      $0_error "cannot create $__p9k_cfg_path_u because ${dir//\%/%%} is readonly"
      return 1
    fi

    [[ ! -e $__p9k_cfg_path || -f $__p9k_cfg_path || -h $__p9k_cfg_path ]] || {
      $0_error "$__p9k_cfg_path_u is a special file"
      return 1
    }
    [[ ! -e $__p9k_zshrc || -f $__p9k_zshrc || -h $__p9k_zshrc ]]          || {
      $0_error "$__p9k_zshrc_u a special file"
      return 1
    }
    [[ ! -e $__p9k_zshrc || -r $__p9k_zshrc ]]                             || {
      $0_error "$__p9k_zshrc_u is not readable"
      return 1
    }
    local style
    for style in lean lean-8colors classic rainbow pure; do
      [[ -r $__p9k_root_dir/config/p10k-$style.zsh ]]                      || {
        $0_error "$__p9k_root_dir_u/config/p10k-$style.zsh is not readable"
        return 1
      }
    done

    (( LINES >= __p9k_wizard_lines && COLUMNS >= __p9k_wizard_columns ))   || {
      $0_error "terminal size too small; must be at least $__p9k_wizard_columns columns by $__p9k_wizard_lines lines"
      return 1
    }
    [[ -t 0 && -t 1 ]]                                                     || {
      $0_error "no TTY"
      return 2
    }
    return 0
  } always {
    unfunction $0_error
  }
}

function p9k_configure() {
  eval "$__p9k_intro"
  _p9k_can_configure || return
  (
    set -- -f
    builtin source $__p9k_root_dir/internal/wizard.zsh
  )
  local ret=$?
  case $ret in
    0)  builtin source $__p9k_cfg_path; _p9k__force_must_init=1;;
    69) return 0;;
    *)  return $ret;;
  esac
}

#### romkatv__powerlevel10k__internal__p10k.zsh

# source: romkatv__powerlevel10k__internal__p10k.zsh
# surface: case arm followed by MATCH-driven replacement expressions

  case $_p9k__cwd in
    /*)
      local parent=/
      local parts=(${(s./.)_p9k__cwd})
    ;;
  esac
  local MATCH
  _p9k__parent_dirs=(${(@)${:-{$#parts..1}}/(#m)*/$parent${(pj./.)parts[1,MATCH]}})
  if ! zstat -A _p9k__parent_mtimes +mtime -- $_p9k__parent_dirs 2>/dev/null; then
    _p9k__parent_mtimes=(${(@)parts/*/-1})
  fi
  _p9k__parent_mtimes_i=(${(@)${:-{1..$#parts}}/(#m)*/$MATCH:$_p9k__parent_mtimes[MATCH]})
  _p9k__parent_mtimes_s="$_p9k__parent_mtimes_i"

#### romkatv__powerlevel10k__internal__parser.zsh

# source: romkatv__powerlevel10k__internal__parser.zsh
# surface: arithmetic conditional with contextual zsh subscript inside a while loop

      while (( c-- > 0 )) || return; do
        token=$tokens[1]
        tokens[1]=()
        if (( $+galiases[$token] )); then
          (( $aln[(eI)p$token] )) && break
          s=$galiases[$token]
          n=p$token
        elif (( e )); then
          break
        fi
      done

#### romkatv__powerlevel10k__internal__wizard.zsh

# source: romkatv__powerlevel10k__internal__wizard.zsh
# surface: nested empty compact function override near function exit

function quit() {
  consume_input
  if [[ $1 == '-c' ]]; then
    print -Pr -- ''
    read -s
  fi
  function quit() {}
  stty echo 2>/dev/null
  show_cursor
  exit 1
}

#### romkatv__powerlevel10k__internal__worker.zsh

# source: romkatv__powerlevel10k__internal__worker.zsh
# surface: anonymous eval callback inside loop body with always follow-through

{
  while zselect -a ready 0 ${(k)_p9k_worker_fds}; do
    [[ $ready[1] == -r ]] || return
    for req in ${(ps:\x1e:)buf}; do
      _p9k_worker_request_id=${req%%$'\x1f'*}
      () { eval $req[$#_p9k_worker_request_id+2,-1] }
      (( $+_p9k_worker_inflight[$_p9k_worker_request_id] )) && continue
      print -rn -- d$_p9k_worker_request_id$'\x1e' || return
    done
  done
} always {
  kill -- -$_p9k_worker_pgid
}

#### zsh-users__zsh-autosuggestions__src__bind.zsh

# source: zsh-users__zsh-autosuggestions__src__bind.zsh
# surface: infix grouped alternatives with trailing wildcard suffix

local -i bind_count

# Save a reference to the original widget
case $widgets[$widget] in
  # Already bound
  user:_zsh_autosuggest_(bound|orig)_*)
    bind_count=$((_ZSH_AUTOSUGGEST_BIND_COUNTS[$widget]))
    ;;

  # User-defined widget
  user:*)
    _zsh_autosuggest_incr_bind_count $widget
    zle -N $prefix$bind_count-$widget ${widgets[$widget]#*:}
    ;;

  # Built-in widget
  *)
    bind_count=0
    ;;
esac

#### zsh-users__zsh-autosuggestions__zsh-autosuggestions.zsh

# source: zsh-users__zsh-autosuggestions__zsh-autosuggestions.zsh
# surface: infix grouped alternatives with trailing wildcard suffix

local -i bind_count

# Save a reference to the original widget
case $widgets[$widget] in
  # Already bound
  user:_zsh_autosuggest_(bound|orig)_*)
    bind_count=$((_ZSH_AUTOSUGGEST_BIND_COUNTS[$widget]))
    ;;

  # User-defined widget
  user:*)
    _zsh_autosuggest_incr_bind_count $widget
    zle -N $prefix$bind_count-$widget ${widgets[$widget]#*:}
    ;;

  # Built-in widget
  *)
    bind_count=0
    ;;
esac

#### zsh-users__zsh-syntax-highlighting__highlighters__main__main-highlighter.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__main-highlighter.zsh
# regression surface: parse error at line 102, column 9: expected 'do'

_zsh_highlight_main_add_many_region_highlights() {
  for 1 2 3; do
    _zsh_highlight_main_add_region_highlight $1 $2 $3
  done
}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-loop.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-loop.zsh
# regression surface: parse error: expected compound command for function body

function b() {} # beware of ALIAS_FUNC_DEF
alias a=b b=c c=b

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-nested-precommand.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-nested-precommand.zsh
# regression surface: parse error: expected compound command for function body

alias a=b b=sudo
sudo(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-precommand-option-argument1.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-precommand-option-argument1.zsh
# regression surface: parse error: expected compound command for function body

# See also param-precommand-option-argument1.zsh
alias sudo_u='sudo -u'
sudo(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-precommand-option-argument2.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-precommand-option-argument2.zsh
# regression surface: parse error: expected compound command for function body

alias sudo_b='sudo -b'
alias sudo_b_u='sudo_b -u'
sudo(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-precommand-option-argument3.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-precommand-option-argument3.zsh
# regression surface: parse error: expected compound command for function body

# See also param-precommand-option-argument3.zsh
alias sudo_u='sudo -u'
sudo(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-precommand-option-argument4.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias-precommand-option-argument4.zsh
# regression surface: parse error: expected compound command for function body

alias sudo_b='sudo -b'
alias sudo_b_u='sudo_b -u'
sudo(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__alias.zsh
# regression surface: parse error: expected compound command for function body

alias alias1="ls"
alias -s alias2="echo"
function alias1() {} # to check that it's highlighted as an alias, not as a function

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__array-cmdsep1.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__array-cmdsep1.zsh
# regression surface: parse error: expected compound command for function body

BUFFER=$'a=( foo | bar )'
bar(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__cmdpos-elision-partial.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__cmdpos-elision-partial.zsh
# regression surface: parse error: expected compound command for function body

sudo(){}
BUFFER=$'$x -u phy1729 ls'

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__commmand-parameter.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__commmand-parameter.zsh
# regression surface: parse error: expected compound command for function body

local x=/usr/bin/env
local y=sudo
local -a z; z=(zsh -f)
sudo(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__off-by-one.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__off-by-one.zsh
# regression surface: parse error: expected compound command for function body

alias a=:
f() {}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__opt-shwordsplit1.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__opt-shwordsplit1.zsh
# regression surface: parse error at line 40, column 2: expected command

#!/usr/bin/env zsh
# -------------------------------------------------------------------------------------------------
# Copyright (c) 2020 zsh-syntax-highlighting contributors
# All rights reserved.
#
# Redistribution and use in source and binary forms, with or without modification, are permitted
# provided that the following conditions are met:
#
#  * Redistributions of source code must retain the above copyright notice, this list of conditions
#    and the following disclaimer.
#  * Redistributions in binary form must reproduce the above copyright notice, this list of
#    conditions and the following disclaimer in the documentation and/or other materials provided
#    with the distribution.
#  * Neither the name of the zsh-syntax-highlighting contributors nor the names of its contributors
#    may be used to endorse or promote products derived from this software without specific prior
#    written permission.
#
# THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR
# IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND
# FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
# CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
# DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
# DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER
# IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT
# OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
# -------------------------------------------------------------------------------------------------
# -*- mode: zsh; sh-indentation: 2; indent-tabs-mode: nil; sh-basic-offset: 2; -*-
# vim: ft=zsh sw=2 ts=2 et
# -------------------------------------------------------------------------------------------------

setopt shwordsplit
local EDITOR='ed -s'

ed() { command ed "$@" }

BUFFER=$'$EDITOR'

expected_region_highlight=(
  '1 7 function' # $EDITOR
)

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__param-precommand-option-argument1.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__param-precommand-option-argument1.zsh
# regression surface: parse error: expected compound command for function body

# See also alias-precommand-option-argument1.zsh
local -a sudo_u; sudo_u=(sudo -u)
sudo(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__param-precommand-option-argument3.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__param-precommand-option-argument3.zsh
# regression surface: parse error: expected compound command for function body

# See also alias-precommand-option-argument3.zsh
local -a sudo_u; sudo_u=(sudo -u)
sudo(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__precommand-unknown-option.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__precommand-unknown-option.zsh
# regression surface: parse error: expected compound command for function body

sudo(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__precommand4.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__precommand4.zsh
# regression surface: parse error: expected compound command for function body

doas(){}
BUFFER=$'doas -nu phy1729 ls'

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-command.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-command.zsh
# regression surface: parse error: expected compound command for function body

sudo(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-comment.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-comment.zsh
# regression surface: parse error: expected compound command for function body

sudo(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-redirection.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-redirection.zsh
# regression surface: parse error: expected compound command for function body

sudo(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-redirection2.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-redirection2.zsh
# regression surface: parse error: expected compound command for function body

sudo(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-redirection3.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__main__test-data__sudo-redirection3.zsh
# regression surface: parse error: expected compound command for function body

sudo(){}

#### zsh-users__zsh-syntax-highlighting__highlighters__pattern__pattern-highlighter.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__pattern__pattern-highlighter.zsh
# regression surface: parse error: unexpected end of input in [[ ]]

_zsh_highlight_pattern_highlighter_loop()
{
  # This does *not* do its job syntactically, sorry.
  local buf="$1" pat="$2"
  local -a match mbegin mend
  local MATCH; integer MBEGIN MEND
  if [[ "$buf" == (#b)(*)(${~pat})* ]]; then
    region_highlight+=("$((mbegin[2] - 1)) $mend[2] $ZSH_HIGHLIGHT_PATTERNS[$pat], memo=zsh-syntax-highlighting")
    "$0" "$match[1]" "$pat"; return $?
  fi
}

#### zsh-users__zsh-syntax-highlighting__highlighters__root__root-highlighter.zsh

# source: zsh-users__zsh-syntax-highlighting__highlighters__root__root-highlighter.zsh
# regression surface: parse error at line 44, column 2: expected command

# root highlighting function.
_zsh_highlight_highlighter_root_paint()
{
  if (( EUID == 0 )) { _zsh_highlight_add_highlight 0 $#BUFFER root }
}

#### zsh-users__zsh-syntax-highlighting__tests__generate.zsh

# source: zsh-users__zsh-syntax-highlighting__tests__generate.zsh
# regression surface: parse error at line 62, column 71: expected command

# Copyright block
year="`LC_ALL=C date +%Y`"
if ! { read -q "?Set copyright year to $year? " } always { echo "" }; then
  year="YYYY"
fi
<$0 sed -n -e '1,/^$/p' | sed -e "s/2[0-9][0-9][0-9]/${year}/" > $fname
# Assumes stdout is line-buffered
git add -- $fname
exec > >(tee -a $fname)

#### zsh-users__zsh-syntax-highlighting__tests__tap-colorizer.zsh

# source: zsh-users__zsh-syntax-highlighting__tests__tap-colorizer.zsh
# surface: wrapped alternatives and wildcard suffixes inside case patterns

while read -r line;
do
  case $line in
    # comment (filename header) or plan
    (#* | <->..<->)
      print -nP %F{blue}
      ;;
    # SKIP
    (*# SKIP*)
      print -nP %F{yellow}
      ;;
    # XPASS
    (ok*# TODO*)
      print -nP %F{red}
      ;;
    *)
      print -nP %F{default}
      ;;
  esac
done

#### zsh-users__zsh-syntax-highlighting__tests__test-highlighting.zsh

# source: zsh-users__zsh-syntax-highlighting__tests__test-highlighting.zsh
# regression surface: parse error at line 135, column 55: expected command

    # WARNING: The remainder of this anonymous function will run with the test's options in effect
    if { ! . "$srcdir"/"$ARG" } || (( $#fail_test )); then
      print -r -- "1..1"
      print -r -- "## ${ARG:t:r}"
      tap_escape $fail_test; fail_test=$REPLY
      print -r -- "not ok 1 - failed setup: $fail_test"
      return ${RETURN:=0}
    fi

#### zsh-users__zsh-syntax-highlighting__tests__test-perfs.zsh

# source: zsh-users__zsh-syntax-highlighting__tests__test-perfs.zsh
# regression surface: parse error at line 103, column 7: expected command

#!/usr/bin/env zsh
# -------------------------------------------------------------------------------------------------
# Copyright (c) 2010-2015 zsh-syntax-highlighting contributors
# All rights reserved.
#
# Redistribution and use in source and binary forms, with or without modification, are permitted
# provided that the following conditions are met:
#
#  * Redistributions of source code must retain the above copyright notice, this list of conditions
#    and the following disclaimer.
#  * Redistributions in binary form must reproduce the above copyright notice, this list of
#    conditions and the following disclaimer in the documentation and/or other materials provided
#    with the distribution.
#  * Neither the name of the zsh-syntax-highlighting contributors nor the names of its contributors
#    may be used to endorse or promote products derived from this software without specific prior
#    written permission.
#
# THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR
# IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND
# FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
# CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
# DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
# DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER
# IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT
# OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
# -------------------------------------------------------------------------------------------------
# -*- mode: zsh; sh-indentation: 2; indent-tabs-mode: nil; sh-basic-offset: 2; -*-
# vim: ft=zsh sw=2 ts=2 et
# -------------------------------------------------------------------------------------------------


# Required for add-zle-hook-widget.
zmodload zsh/zle

# Check an highlighter was given as argument.
[[ -n "$1" ]] || {
  echo >&2 "Bail out! You must provide the name of a valid highlighter as argument."
  exit 2
}

# Check the highlighter is valid.
[[ -f ${0:h:h}/highlighters/$1/$1-highlighter.zsh ]] || {
  echo >&2 "Bail out! Could not find highlighter ${(qq)1}."
  exit 2
}

# Check the highlighter has test data.
[[ -d ${0:h:h}/highlighters/$1/test-data ]] || {
  echo >&2 "Bail out! Highlighter ${(qq)1} has no test data."
  exit 2
}

# Load the main script.
typeset -a region_highlight
. ${0:h:h}/zsh-syntax-highlighting.zsh

# Activate the highlighter.
ZSH_HIGHLIGHT_HIGHLIGHTERS=($1)

# Runs a highlighting test
# $1: data file
run_test_internal() {
  local -a highlight_zone

  local tests_tempdir="$1"; shift
  local srcdir="$PWD"
  builtin cd -q -- "$tests_tempdir" || { echo >&2 "Bail out! cd failed: $?"; return 1 }

  # Load the data and prepare checking it.
  PREBUFFER= BUFFER= ;
  . "$srcdir"/"$1"

  # Check the data declares $PREBUFFER or $BUFFER.
  [[ -z $PREBUFFER && -z $BUFFER ]] && { echo >&2 "Bail out! Either 'PREBUFFER' or 'BUFFER' must be declared and non-blank"; return 1; }

  # Set $? for _zsh_highlight
  true && _zsh_highlight
}

run_test() {
  # Do not combine the declaration and initialization: «local x="$(false)"» does not set $?.
  local __tests_tempdir
  __tests_tempdir="$(mktemp -d)" && [[ -d $__tests_tempdir ]] || {
    echo >&2 "Bail out! mktemp failed"; return 1
  }
  typeset -r __tests_tempdir # don't allow tests to override the variable that we will 'rm -rf' later on

  {
    (run_test_internal "$__tests_tempdir" "$@")
  } always {
    rm -rf -- "$__tests_tempdir"
  }
}

# Process each test data file in test data directory.
local data_file
TIMEFMT="%*Es"
{ time (for data_file in ${0:h:h}/highlighters/$1/test-data/*.zsh; do
  run_test "$data_file"
  (( $pipestatus[1] )) && exit 2
done) } 2>&1 || exit $?

exit 0

#### zsh-users__zsh-syntax-highlighting__tests__test-zprof.zsh

# source: zsh-users__zsh-syntax-highlighting__tests__test-zprof.zsh
# regression surface: parse error at line 78, column 9: expected command

#!/usr/bin/env zsh
# -------------------------------------------------------------------------------------------------
# Copyright (c) 2010-2015 zsh-syntax-highlighting contributors
# All rights reserved.
#
# Redistribution and use in source and binary forms, with or without modification, are permitted
# provided that the following conditions are met:
#
#  * Redistributions of source code must retain the above copyright notice, this list of conditions
#    and the following disclaimer.
#  * Redistributions in binary form must reproduce the above copyright notice, this list of
#    conditions and the following disclaimer in the documentation and/or other materials provided
#    with the distribution.
#  * Neither the name of the zsh-syntax-highlighting contributors nor the names of its contributors
#    may be used to endorse or promote products derived from this software without specific prior
#    written permission.
#
# THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR
# IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND
# FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
# CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
# DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
# DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER
# IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT
# OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
# -------------------------------------------------------------------------------------------------
# -*- mode: zsh; sh-indentation: 2; indent-tabs-mode: nil; sh-basic-offset: 2; -*-
# vim: ft=zsh sw=2 ts=2 et
# -------------------------------------------------------------------------------------------------

# Load the main script.
typeset -a region_highlight
. ${0:h:h}/zsh-syntax-highlighting.zsh

# Activate the highlighter.
ZSH_HIGHLIGHT_HIGHLIGHTERS=(main)

source_file=0.7.1:highlighters/$1/$1-highlighter.zsh

# Runs a highlighting test
# $1: data file
run_test_internal() {
  setopt interactivecomments

  local -a highlight_zone

  local tests_tempdir="$1"; shift
  local srcdir="$PWD"
  builtin cd -q -- "$tests_tempdir" || { echo >&2 "Bail out! cd failed: $?"; return 1 }

  # Load the data and prepare checking it.
  PREBUFFER=
  BUFFER=$(cd -- "$srcdir" && git cat-file blob $source_file)
  expected_region_highlight=()

  zmodload zsh/zprof
  zprof -c
  # Set $? for _zsh_highlight
  true && _zsh_highlight
  zprof
}

run_test() {
  # Do not combine the declaration and initialization: «local x="$(false)"» does not set $?.
  local __tests_tempdir
  __tests_tempdir="$(mktemp -d)" && [[ -d $__tests_tempdir ]] || {
    echo >&2 "Bail out! mktemp failed"; return 1
  }
  typeset -r __tests_tempdir # don't allow tests to override the variable that we will 'rm -rf' later on

  {
    (run_test_internal "$__tests_tempdir" "$@")
  } always {
    rm -rf -- "$__tests_tempdir"
  }
}

run_test

#### zsh-users__zsh-syntax-highlighting__zsh-syntax-highlighting.zsh

# source: zsh-users__zsh-syntax-highlighting__zsh-syntax-highlighting.zsh
# surface: compact empty helper function before zle hook registration

if is-at-least 5.9 && _zsh_highlight__function_callable_p add-zle-hook-widget
then
  _zsh_highlight__zle-line-pre-redraw() {
    true && _zsh_highlight "$@"
  }
  _zsh_highlight_bind_widgets(){}
  if [[ -o zle ]]; then
    add-zle-hook-widget zle-line-pre-redraw _zsh_highlight__zle-line-pre-redraw
    add-zle-hook-widget zle-line-finish _zsh_highlight__zle-line-finish
  fi
else
  _zsh_highlight_bind_widgets() {
    zmodload zsh/zleparameter 2>/dev/null || {
      print -r -- >&2 failed
      return 1
    }
  }
fi

#### minimization__zsh_parameter_quoted_word_target_trim

# regression surface: quoted command-substitution targets in zsh parameter trims

print ${"$(xcode-select -p)"%%/Contents/Developer*}

#### minimization__zsh_parameter_nested_length_target

# regression surface: nested zsh replacement expressions under ${#...} length prefixes

print ${#${cd//${~q}/}}

#### minimization__zsh_parameter_colon_modifier_targets

# regression surface: zsh :h/:t/:l-style suffixes on identifier and positional targets

print ${REPLY:l} ${1:t} ${0:h}
