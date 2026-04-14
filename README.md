# My daily driver changes for Helix
The branch names are listed with each item.

- (modeline-support) Modeline support. Tries vi then emacs before falling back to the usual detection methods.
- (text-fallback) "text" is defined as the fallback when no other language is detected. Why? The next item means now LSPs will
  load for plain files with no extension. Or new files that are unknown. Have not setup foo.lang just yet? You
  at least get whatever the base LSPs you have configured like a spell checker.
- (lsp-path-remapping-v2) Support for remapping paths to enable remote Language Servers (aka LSP)
- (add-global-language-config) support for a `[global]` section in languages.toml. This allows language servers to be specified which apply to
  all languages instead of needing to add a common server to each and every language. `inherit-global-language-servers: false`
  will opt a specific language out of this if required. If a language specifies the same language server but with
  different options it wins over the global one.
- (yank-history) Yank history. Each register has a history of the last 10 yanks. These can be retrieved using a picker that allows
  searching within the yanked text. Helpful for whole paragraphs or functions being moved around. By default yanking,
  deleting, and changing all update the history. But you can use the `_noyank` versions if you prefer. Personally, I have
  'd' and 'c' set to noyank and only actually yank with 'y'. But I have Alt-d and Alt-c defined when I need them. Alt+p
  and Alt+P are the same as normal paste but use the history picker.
- (conflict-sorting) changed file picker now sorts with conflicted items at the top and untracked at the bottom
- (theme-preview) theme-preview command. Show all of the system defined theme elements in a picker. Handy for to quickly explore themes. Or see how a theme renders a
  specific element.
- (github-view-command) ghv command. This runs `ghv` from your path and passed the current filename and line number. ghv stands for "github view"
  and is intended to load the file from the current repo on GitHub. Useful to sending links to co-workers or contributors.
  Command is not included. My current version is quite simplistic: [ghv](https://github.com/shaleh/useful-things/blob/main/scripts/ghv)

A [Kakoune](https://github.com/mawww/kakoune) / [Neovim](https://github.com/neovim/neovim) inspired editor, written in Rust.

The editing model is very heavily based on Kakoune; during development I found
myself agreeing with most of Kakoune's design decisions.

For more information, see the [website](https://helix-editor.com) or
[documentation](https://docs.helix-editor.com/).

All shortcuts/keymaps can be found [in the documentation on the website](https://docs.helix-editor.com/keymap.html).

[Troubleshooting](https://github.com/helix-editor/helix/wiki/Troubleshooting)

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
