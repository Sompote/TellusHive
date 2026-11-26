# 10. Command line, data and troubleshooting

This chapter lists the commands that run TellusHive from a terminal, where it keeps its files, and what to do when something does not work.

## Commands

The same program opens the window or runs in a terminal, depending on how you start it. On macOS the program inside the app is `/Applications/TellusHive.app/Contents/MacOS/tellushive`, and on Windows it is `tellushive.exe`.

```sh
tellushive                                    # the window
tellushive run --project DIR "Design a ..."   # one turn of the project's team, printed in the terminal
tellushive plugins [--project DIR]            # the plugin tree and each plugin's state
tellushive --mcp --project DIR                # the engineering tools as an MCP server on stdio
tellushive serve [--port 8796] [--token T]    # the bots over HTTP, for the iPhone app
```

`run` continues the same conversation as the window's for that project's team and prints every step, the files made and a token report at the end, so it suits scripted checks. `--mcp` offers TellusHive's own tools (calculations, the standards library and the rest) to another agent such as Claude Code, which is different from the bots' MCP server described in [chapter 6](06-plugins-and-mcp.md).

## Where TellusHive keeps its files

The data folder holds what is shared by all projects. On macOS it is `~/Library/Application Support/TellusHive`, on Windows `%APPDATA%\TellusHive`, and on Linux `~/.local/share/TellusHive`, and the environment variable `TELLUSHIVE_HOME` moves it anywhere else. It contains `settings.json` (providers, keys, plugins and limits, readable only by you), `bots/` with one folder per bot (its settings, picture and, by default, its own workspace), `teams/`, `templates/`, `standards/` (the indexed library) and `skills/`.

Each project is its own folder, under `~/Documents/TellusHive` by default ([chapter 4](04-projects.md)). Its `sessions/` folder holds one log per agent session in JSON Lines, which records every message, tool call and result, and it is the place to look when you want to know exactly what an agent did. Starting a new conversation keeps the old logs.

## Troubleshooting

**An agent says there is no API key.** Add the key for the agent's provider under **Settings › Models**, or set its environment variable before you start TellusHive, or choose `claude-code` or `codex` for the agent if you are signed in to one of them. **List models** on the provider's card checks the address and the key.

**A plugin shows failed.** Open **Settings › Plugins & MCP**, select it, and read the red message and the **Server log**. The usual causes are a program that is not at the path given (install it and click **Find installed programs again**, or correct **Program**), a missing `--mcp` argument, or an HTTP endpoint that needs a header such as `Authorization`. A plugin you do not have can simply be switched off.

**The agents cite nothing, or cite the wrong standard.** Check that the project follows the right standards under **Project › Standards**, and try the same question in the search box there. If the library has no matching clause, import the standard, and if the clauses come back badly split, import a cleaned-up Markdown copy ([chapter 5](05-standards-library.md)).

**A run takes too long.** Turn thinking off for routine agents, give each agent only the tools and plugins it uses, set a **Time limit** per agent in **Team**, and check the limits under **Settings › General**. A bot that works alone answers small questions much faster than a whole team.

**A link in the chat does not open.** The chat opens only files inside the project folder that exist, and it never opens programs or scripts. The message above the message box says why a link was refused, and you can always open the file from the project folder with **Show in Finder**.

**New conversation is greyed out.** The bot is still working. Stop the task with **Stop** in the top bar, then start the new conversation.

**Something else.** Report it at [github.com/Sompote/TellusHive/issues](https://github.com/Sompote/TellusHive/issues) with what you did, what you expected and what happened, and attach the relevant lines of the session log if you can.
