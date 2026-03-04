<div align="center">

<h1>
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="logo_dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="logo_light.svg">
  <img alt="Helix" height="128" src="logo_light.svg">
</picture>
</h1>

[![Build status](https://github.com/helix-editor/helix/actions/workflows/build.yml/badge.svg)](https://github.com/helix-editor/helix/actions)
[![GitHub Release](https://img.shields.io/github/v/release/helix-editor/helix)](https://github.com/helix-editor/helix/releases/latest)
[![Documentation](https://shields.io/badge/-documentation-452859)](https://docs.helix-editor.com/)
[![GitHub contributors](https://img.shields.io/github/contributors/helix-editor/helix)](https://github.com/helix-editor/helix/graphs/contributors)
[![Matrix Space](https://img.shields.io/matrix/helix-community:matrix.org)](https://matrix.to/#/#helix-community:matrix.org)

</div>

![Screenshot](./screenshot.png)

A [Kakoune](https://github.com/mawww/kakoune) / [Neovim](https://github.com/neovim/neovim) inspired editor, written in Rust.

The editing model is very heavily based on Kakoune; during development I found
myself agreeing with most of Kakoune's design decisions.

For more information, see the [website](https://helix-editor.com) or
[documentation](https://docs.helix-editor.com/).

All shortcuts/keymaps can be found [in the documentation on the website](https://docs.helix-editor.com/keymap.html).

[Troubleshooting](https://github.com/helix-editor/helix/wiki/Troubleshooting)

# My daily driver changes for Helix

- Support for a `[global]` section in languages.toml. This allows language servers to be specified which apply to
  all languages instead of needing to add a common server to each and every language. `inherit-global-language-servers: false`
  will opt a specific language out of this if required. If a language specifies the same language server but with
  different options it wins over the global one.
- ghv command. This runs `ghv` from your path and passed the current filename and line number. ghv stands for "github view"
  and is intended to load the file from the current repo on GitHub. Useful to sending links to co-workers or contributors.
  Command is not included. My curernt version is quite simplistic:
  ```
  #!/bin/sh

  ###
  # Simple script to load Github and view a file.
  ###
  
  REPO_PATH="$HOME/repos"
  BRANCH="${BRANCH:-main}"
  
  make_absolute() {
      filename=$1
  
      # Step 1. Ensure the filename is absolute. This ensures that even when called from
      # deep in the tree the paths work.
      case "$filename" in
          /*)
              # Absolute already, do nothing.
              echo "$filename"
              ;;
          *)
              echo "$PWD/$filename"
              ;;
      esac
  }
  
  strip_local_repo_path() {
      filename=$1
      for base in 'work' 'personal' 'opensource'; do
          # Now, a fancy pattern replace to strip off the absolute prefix.
          stripped="${filename#"${REPO_PATH}/${base}/"}"
          case "$stripped" in
              /*)
                  # Still has leading slash. This prefix did not match.
                  ;;
              *)
                  # Clean, return it.
                  echo "$stripped"
                  return
                  ;;
          esac
      done
  
      echo "No prefixes matched. Fail!" >/dev/stderr
      exit 1
  }
  
  # These are optional. Will simply load the source tree if left out.
  filename=$1
  linenumber=$2
  
  if [ -z "$filename" ]; then
      BASE_URL="$(git remote get-url origin)"
      if [ -z "$URL" ]; then
          echo "Don't know what Github URL to load. Move into a git repo with an origin defined." >/dev/stderr
          exit 1
      fi
      URL="${BASE_URL}/tree/${BRANCH}/"
  else
      filename=$(make_absolute "$filename")
      BASE_URL="$(cd "$(dirname "$filename")" && git remote get-url origin)"
      filename=$(strip_local_repo_path "$filename")
      # Now pop off the next directory. This should be the repo name but this clone could have been renamed.
      filename="${filename#*/}"
  
      URL="${BASE_URL}/blob/${BRANCH}/${filename}"
  
      if [ ! -z "$linenumber" ]; then
          URL="${URL}#L${linenumber}"
      fi
  fi
  
  echo "$URL"
  
  case "$(uname -s)" in
      Darwin)
          CMD="open"
          ;;
      *)
          CMD="xdg-open"
          ;;
  esac
  $CMD "$URL"
  ```

# Features

- Vim-like modal editing
- Multiple selections
- Built-in language server support
- Smart, incremental syntax highlighting and code editing via tree-sitter

Although it's primarily a terminal-based editor, I am interested in exploring
a custom renderer (similar to Emacs) using wgpu.

Note: Only certain languages have indentation definitions at the moment. Check
`runtime/queries/<lang>/` for `indents.scm`.

# Installation

[Installation documentation](https://docs.helix-editor.com/install.html).

[![Packaging status](https://repology.org/badge/vertical-allrepos/helix-editor.svg?exclude_unsupported=1)](https://repology.org/project/helix-editor/versions)

# Contributing

Contributing guidelines can be found [here](./docs/CONTRIBUTING.md).

# Getting help

Your question might already be answered on the [FAQ](https://github.com/helix-editor/helix/wiki/FAQ).

Discuss the project on the community [Matrix Space](https://matrix.to/#/#helix-community:matrix.org) (make sure to join `#helix-editor:matrix.org` if you're on a client that doesn't support Matrix Spaces yet).

# Credits

Thanks to [@jakenvac](https://github.com/jakenvac) for designing the logo!
