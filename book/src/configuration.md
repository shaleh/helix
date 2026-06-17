# Configuration

To override global configuration parameters, create a `config.toml` file located in your config directory:

- Linux and Mac: `~/.config/helix/config.toml`
- Windows: `%AppData%\helix\config.toml`

> 💡 You can easily open the config file by typing `:config-open` within Helix normal mode.

Example config:

```toml
theme = "onedark"

[editor]
line-number = "relative"
mouse = false

[editor.cursor-shape]
insert = "bar"
normal = "block"
select = "underline"

[editor.file-picker]
hidden = false
```

You can use a custom configuration file by specifying it with the `-c` or
`--config` command line argument, for example `hx -c path/to/custom-config.toml`.
You can reload the config file by issuing the `:config-reload` command. Alternatively, on Unix operating systems, you can reload it by sending the USR1
signal to the Helix process, such as by using the command `pkill -USR1 hx`.

Finally, you can have a `config.toml` and a `languages.toml` local to a project by putting it under a `.helix` directory in your repository.
Its settings will be merged with the configuration directory and the built-in configuration.

## `[commands]`

The `[commands]` table lets you define typable command aliases — names you
type at the `:` prompt that expand to one or more built-in command lines.
Aliases are useful for shortening verbose commands or saving a known-good
argument set you can't remember:

```toml
[commands]
# Shorthand:
":q" = ":quit"

# A saved argument set you'd otherwise re-type:
":show-blame" = ":set-option gutters [\"diagnostics\", \"spacer\", \"line-numbers\", \"spacer\", \"blame\"]"

# Sequence of commands, run in order:
":wq" = [":write", ":quit"]

# Unprefixed key — hidden from completion but still invokable as `:scratch`:
"scratch" = ":new"
```

### Visibility

A leading `:` on the alias key makes the alias appear in command-mode
tab-completion. An unprefixed key is hidden but remains invokable when typed
in full. The leading `:` is **not** part of the invocation — you type
`:show-blame` either way.

### Variable expansion

Aliases inherit Helix's universal variable expansion. The body of an alias
can use `%{...}` variables (e.g. `%{buffer_name}`), `%sh{...}` for shell
commands, and `%reg{x}` for register contents — expansion happens at
invocation time, not at config load:

```toml
[commands]
":rm" = ":sh rm %{buffer_name}"
":lg" = ":sh tmux popup -E lazygit"
```

### Shadowing built-ins

If an alias key matches a built-in command name (or one of its built-in
aliases), the alias wins. There is no escape mechanism in this fork — if you
need the built-in back, rename your alias.

### Recursion

Aliases may expand into other aliases. Recursion is capped at depth 8.
Exceeding the cap surfaces an error and aborts the chain, so cycles fail
loudly rather than hanging the editor.

### Deliberately omitted

This fork supports a deliberate subset of the upstream proposal
([helix-editor/helix#12320](https://github.com/helix-editor/helix/pull/12320)).
The following are rejected at config load with an explicit message:

- **Positional argument placeholders** (`%arg{0}`, `%arg{1}`, ...) inside
  alias bodies.
- **Inline-table form** (`[commands.":foo"]` with `desc`, `accepts`,
  `completer` fields).

The TOML shape for the supported subset (string form, array form,
leading-`:` visibility convention) matches PR #12320 exactly, so a future
move to upstream's full feature requires no config edits.

