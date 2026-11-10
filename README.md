<p align="center"><img src="app/assets/app_icon.png" width="112" alt="TellusHive icon"></p>

# TellusHive

TellusHive is an open-source multi-agent system for civil-engineering design, written entirely
in Rust. The engineer chats with a coordinator agent, which hands the work to a team of
specialists that calculate to the chosen standards, check each other's results, draw, write the
calculation report and drive FE programs such as Tellus through MCP.

Its agent harness, `th-hive`, is developed from [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness)
(`dsh`, DeepSeek-AI) and ports its **everything-is-a-plugin** architecture to Rust. Like dsh,
TellusHive builds on [Cordis](https://github.com/cordiverse/cordis), whose design is described in
[_A Programming Paradigm for Spatiotemporal Composability_](https://arxiv.org/abs/2608.25512): the
model adapter, the tool registry, the session log and the agent loop are all plugins on one
shared context, and the engineering side of TellusHive is one more bundle of plugins on top.

Documentation: [architecture](docs/architecture.md) · [user guide](docs/guide.md)

## Developer preview

TellusHive is in _developer preview_ and changes quickly. **THERE WILL BE COMPATIBILITY-BREAKING
CHANGES**, including to the session log format and the settings file.

Review the [safety notice](SAFETY.md) before running the project, because its agents can run code
and drive programs on your computer.

## Run

### Run from source

Install [Rust](https://rustup.rs), then run:

```sh
git clone https://github.com/Sompote/TellusHive.git
cd TellusHive
cargo build --release
./target/release/tellushive
```

The last command opens the window. Add a provider key in **Settings**, or set
`OPENROUTER_API_KEY` or `DEEPSEEK_API_KEY` before starting it. The [user guide](docs/guide.md)
explains the workspaces, the team canvas, templates and the engineering programs.

### Other commands

```sh
./target/release/tellushive run --project DIR "Design a ..."   # one turn in the terminal
./target/release/tellushive plugins [--project DIR]            # the plugin tree (like dsh --dump-config)
./target/release/tellushive --mcp --project DIR                # the engineering tools as an MCP server
```

## Development

Start with the [architecture documentation](docs/architecture.md), which maps each part of
`th-hive` to the DeepSeek Harness package it is developed from and shows the turn flow, the tool
pipeline and the session log.

```sh
cargo test --workspace     # 80 tests, no API key needed
```

The harness tests replace the model provider with a scripted adapter, so they cover the step
loop, parallel delegation, retry, compaction, spill, the task board and resuming a session
without any key. New behaviour goes into a plugin that attaches to a documented seam (a tool,
a tool source, a prompt section, a subagent source or a hook) rather than into the loop itself.

## Citation

If you use TellusHive, please also cite DeepSeek Harness, whose architecture its harness follows:

```bibtex
@misc{deepseek-harness2026,
  title={DeepSeek Harness: Everything is a Plugin},
  author={DeepSeek-AI},
  year={2026},
  publisher={GitHub},
  howpublished={\url{https://github.com/deepseek-ai/deepseek-harness}},
}
```

## License

[MIT](LICENSE)

TellusHive is not affiliated with or endorsed by DeepSeek. The DeepSeek Harness licence is
reproduced in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
