# TellusHive user guide

This guide covers the window, token reporting, templates, skills and plugins, the engineering
programs TellusHive drives, CADCraft, and where TellusHive keeps its data. For the design of the
agent harness see [architecture.md](architecture.md).

## Workspaces in the window

- **Design** has the project explorer on the left (description, notes for every agent, chosen
  standards, team, artifacts), the chat in the middle and the artifact viewer on the right. The
  viewer shows calculation sheets with PASS/FAIL, drawings, reports and pictures, and files can
  be attached with the button or dropped onto the window. While an agent writes, the panel of
  agents at work shows its text as it streams.
- **Team** shows the agents as nodes on a canvas. Each agent has its own role, instructions,
  provider and model, built-in tools, skills, plugins, delegation targets, and a switch for
  whether it may use Claude Code or Codex. Teams are saved as JSON in the data folder.
- **Standards** imports standards (PDF, txt, md) into the vector library and sets which ones
  the project follows. You can add office rules as searchable notes and test a search there.
- **Plugins** manages MCP servers. Tellus and Tellus3D are added automatically when their
  programs are found, and any stdio program or HTTP endpoint can be added and connected later.
- **Settings** holds the providers and keys, the default and vision models, the embedding
  model, Claude Code and Codex, the skill folders, the projects folder and the theme.

## Claude Code or Codex as an agent's model

Any agent can use Claude Code or Codex as its model instead of a model API: choose the provider
`claude-code` or `codex` for it in the **Team** step, or make one of them the default provider
in **Settings**. The agent then runs on your Claude or ChatGPT subscription and needs no API
key, and the switch "Use my subscription" (on by default, under External agents) removes
`ANTHROPIC_API_KEY` or `OPENAI_API_KEY` from the program's environment so a key in your shell
does not override the login. The model name is optional: `sonnet`, `opus` or `haiku` for Claude
Code, or empty for the program's default.

One step of such an agent is one run of the program. It receives the agent's prompt and
conversation on stdin, together with this program's MCP server started for that agent
(`tellushive --mcp --agent <id>`), so it calls exactly the tools an API model would have: the
built-in design tools, the standards library, the agent's plugins, `delegate` to its team
members, `todo_write` and the task board. Every call runs through the th-hive tool pipeline and
is logged in a session of its own (`sessions/<agent>-tools-*.jsonl`), and the calls are copied
into the agent's session as they happen, so the chat shows them like any other tool call. A
team can mix providers freely, for example a coordinator on Claude Code that delegates to a
designer on DeepSeek. Measured on one short question, the coordinator answered in 10 s on
Claude Code and 14 s on Codex. Each Claude Code step carries the program's own system prompt
and tools, about 116k input tokens, most of them read from its cache, and Codex runs report no
token counts.

## Token use

The window counts tokens while the agents work. The top bar shows the running total of the task
next to the timer, the chat shows the model calls and the tokens in and out so far, and the
status bar shows the project's total. When a task ends, a card in the chat reports whether it
finished, was stopped or failed, how long it took and the totals, and it opens into a table by
agent and model with each agent's share. The headless `run` command prints the same report. The
counts cover the agents' model calls only, so picture descriptions by the vision model,
embeddings, compaction summaries and work done inside Claude Code or Codex are not included, and
no cost is shown because the app holds no prices. Each `assistant/message` in the session log
also records the prompt tokens the provider served from its cache.

## Templates

A template saves the whole set-up of the agentic system under a name, so a new project can start
from it. It holds the team (each agent's role, instructions, model, tools, skills, plugins and
delegation links), the chosen standards, the project description and notes, the default and
vision models, the plugins the team uses and the skill folders. API keys are never saved, and
plugin environment values and HTTP headers are blanked, while the chat and the outputs stay
with their project. Save a template in the **Project** step (or from the project menu), and
start a project from it, export it or delete it on the **Home** page, where **Import
template…** reads a template file from another computer. Plugins missing there are added but
switched off until their program is found, and standards missing from the library are listed
for import.

## Skills and plugins

A skill is a folder that holds a `SKILL.md` (front matter with `name` and `description`) and its
reference files. This is the Claude skills format, so `~/.claude/skills` is scanned by default.
An agent sees the one-line description of each skill it is equipped with and loads the full text
with `read_skill`. Plugins are MCP servers, and an agent equipped with a plugin sees that
plugin's tools as `<plugin>__<tool>`.

## The Tellus family and other programs

Each program's MCP server (`--mcp`) is added as a plugin automatically. The finder looks at the
installed app and the build folders and uses the newest copy, and "Find installed programs
again" in step 4 repeats the search after an install.

| Plugin | Program | Tools (10 Nov 2026) | Agent |
|---|---|---:|---|
| `tellus` | Tellus 2D geotechnical FE | 37 | Geotechnical analyst |
| `tellus3d` | Tellus3D 3D geotechnical FE | 31 | Geotechnical analyst |
| `motus` | Motus 1-D site response | 26 | Geotechnical analyst |
| `pondus` | Pondus structural analysis and design | 47 | Structural analyst |
| `continuax` | ContinuaX structural FE | 34 | Structural analyst |
| `granum` | Granum DEM | 33 | Particle modeller |
| `granumfl` | GranumFL DEM-SPH | 53 | Particle modeller |
| `aedifex` | Aedifex BIM modeller | 50 | BIM modeller |
| `cadcraft` | CADCraft CAD | 15 | Drafter, BIM modeller |

The programs are split across specialist agents because providers accept at most 128 tools per
request and every tool description is sent with every model call. In th-hive the programs of an
agent are a per-agent tool source, so when an agent's tools would exceed 128 the largest
plugins are reached through two gateway tools instead of being listed one by one:
`plugin_tools` lists a plugin's tools or gives one tool's argument schema, and `plugin_call`
runs it, so no tool is lost. The coordinator holds no plugins, because it delegates the analyses
to the specialists.

## CADCraft

Every drawing is saved as SVG (screen and report), DXF R12 (CAD, true scale) and JSON. The
viewer's **Open in CADCraft** button loads the DXF into the CADCraft window through CADCraft's
MCP bridge (`cadcraft-cli mcp --connect 127.0.0.1:7979 --lazy --launch`), starting CADCraft when
it is not running, so quit CADCraft first if it is already open without its control port. The
Drafter is equipped with the `cadcraft` plugin, so it can also open drawings in CADCraft and
edit them with AutoCAD-style commands when the engineer asks. In the DXF, pattern hatches sit on
their own grey `HATCH` layer and black or dark-grey ink uses ACI colour 7, so drawings read on
CAD's dark screen and print black.

## Data locations

- `~/Library/Application Support/TellusHive/`: `settings.json` (owner-only, holds keys and the
  `harness` block), `teams/`, `templates/`, `standards/` (the vector library), `skills/`.
- `~/Documents/TellusHive/<project>/`: `project.json`, `attachments/`, `calcs/`, `drawings/`,
  `reports/`, `images/`, `work/`, `chat.json` (the transcript shown in the window) and
  `sessions/`. The `sessions/` folder holds one JSONL log per agent session, the pointer
  `current` to the coordinator session the chat continues, and `spill/` for long tool outputs.
  "Clear the conversation" starts a new session and keeps the old files.

A project made with the earlier Agentic Design app opens normally, but its conversation history
in `chat.json` is not carried over into a session, so the agents start that project's chat
afresh.
