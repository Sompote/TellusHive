# Safety notice

TellusHive agents act on your computer, not only in a chat. Read this page before you run the
project, and run it only on projects and files you are willing to let the agents change.

## What the agents can do

The agents write files inside the project folder (calculation sheets, drawings, reports and
anything under `work/`), and the `run_python` tool runs Python code that a model wrote, with
your user's rights and without a sandbox. When Claude Code or Codex is switched on, the
coordinator can start them in the project's `work/` folder, by default with permission to edit
files there (`--permission-mode acceptEdits` for Claude Code, `--sandbox workspace-write` for
Codex), so check these arguments in **Settings** before you allow external agents. Agents
equipped with MCP plugins drive those programs (Tellus, Tellus3D, Pondus, Aedifex, CADCraft and
others) through every tool the program exposes, including tools that save or overwrite its model
file.

Unlike DeepSeek Harness, TellusHive has no process sandbox and no approval prompt yet. Its tool
pipeline has the `ask` decision and the `approval` service, but no window answers them, so a call
that asks for approval is refused rather than run.

## What leaves your computer

Each model request sends the system prompt, the conversation history and tool results to the
provider you chose (OpenRouter, DeepSeek, OpenAI or a local Ollama server), so project data,
standards excerpts and attached files reach that provider. Pictures are sent to the vision model
when one is set up. TellusHive does not upload its session logs anywhere, and the DeepSeek
Harness extension that sends session logs to DeepSeek is deliberately not ported.

## Engineering results

The agents can be wrong. A recorded test run showed both the designer and the checker treating a
given support shear as the shear at distance d, which led them to report a load inconsistency
that did not exist. The default team now tells the coordinator, designer and checker to read a
given Vu as the support reaction unless the engineer says otherwise, but every design must be checked by a qualified engineer before use, and clause
citations are only as good as the standards imported into the library.

## Keys

API keys are stored in `settings.json`, which is created readable by your user only, and are
never written to templates, session logs or this repository. Rotate any key that has appeared in
a chat, a log or a terminal transcript.
