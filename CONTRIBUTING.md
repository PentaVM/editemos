# Contributing

Thanks for looking. editemos is one Rust crate with no build scripts and no async runtime.

```bash
cargo build
cargo test          # browser and ffmpeg tests skip themselves when those are missing
cargo fmt --all
cargo clippy --all-targets -- -D warnings
```

## Ground rules

- **One operation, two front ends.** Anything a user can do lives in `src/ops.rs` as a function
  from JSON to JSON. The command line (`src/main.rs`) and the MCP server (`src/mcp.rs`) only
  translate. If you add a capability, add it there and expose it in both.
- **Nothing leaves the machine by default.** Network access is limited to what the user asked
  for: reading a website they named, syncing skills they listed, calling the model they chose.
- **Never write to stdout from library code.** `editemos serve` speaks JSON-RPC on stdout;
  a stray `println!` breaks every client. Use `util::trace` (stderr, opt-in).
- **Do not vendor third-party skills.** Add an entry to `skills/catalog.toml` with a permissive
  licence you have checked in the repository's own LICENSE file.
- **Test what you claim.** A render test decodes a frame and looks at its alpha. A clips test
  measures the output frame. Follow that standard.

## Adding a motion template

1. Put `templates/<kind>.html` next to the others. It must read the edit-brand tokens from the
   prelude (`D`, `E`, `C`, `TY`, `HOLD`, `ST`) rather than hard-coding durations, curves or colours.
2. Register it in `src/motion/templates.rs` (`KINDS` and `tpl`).
3. `editemos motion new <kind> "text"`, then `motion lint` and `motion render`, and look at the
   contact sheet for two different projects: it must not look the same in both.

## Adding an adapter

Most models need no code: add a block to `~/.editemos/adapters.toml` (`kind = "exec"` for a
command-line agent, `kind = "openai-chat"` for any OpenAI-compatible endpoint). Built-in defaults
live in `src/adapters.rs`.
