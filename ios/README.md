# TellusHive for iPhone

The iPhone app is a native SwiftUI client for the bots that TellusHive runs on a cloud machine. The bots, their teams, the standards library and the analysis programs all stay on the server, because the agent harness starts MCP programs and works on project folders, which an iPhone cannot do. The phone sends messages, photos and files, follows each task live as the bot writes, and opens the calculations, drawings and reports the team produces in Quick Look. A task runs on the server whether or not the phone stays connected, so you can lock the phone during a long design run and the app picks the conversation up again when you return.

## 1. Run the server on the cloud machine

The server is the `serve` mode of the normal `tellushive` binary, so it needs no separate build. On the cloud machine, clone the repository and run `cargo build --release`. On a fresh Linux image the build may stop on a missing system library for the desktop window, and the error names the package to install (on Debian or Ubuntu it is usually `build-essential` and `pkg-config` plus one or two `-dev` packages). The server then needs the same data folder the desktop app uses, which holds `settings.json` with the provider API keys, the `bots/` and `teams/` folders and the `standards/` library. On macOS this folder is `~/Library/Application Support/TellusHive`, and on Linux it is `~/.local/share/TellusHive`, so copying the folder from your Mac to that place on the server moves your bots across. Paths written inside `settings.json` (the projects folder, skill folders and the commands of plugins such as Tellus or Aedifex) point at your Mac, so edit them for the server or switch those plugins off there. Set `TELLUSHIVE_HOME` instead if you want the data folder somewhere else.

```sh
./target/release/tellushive serve            # listens on 127.0.0.1:8796
```

On first start the server writes a random token to `server-token` in the data folder and prints it, and the phone needs this token. You can also set it yourself with `--token` or the `TELLUSHIVE_SERVER_TOKEN` variable. To keep the server running after you log out and after a reboot, install `tellushive.service` from this folder as a systemd unit, after setting its `User` and binary path.

The server listens on `127.0.0.1` by default because it speaks plain HTTP. The token travels in every request, so it must only cross the internet inside HTTPS, which the next step provides.

## 2. Give it an HTTPS address

Tailscale is the simplest and safest choice, because the server stays off the public internet and only your own devices can reach it. Install Tailscale on the server and on the iPhone, sign both into the same account, and run this on the server:

```sh
sudo tailscale serve --bg 8796
```

Tailscale prints an address such as `https://tellus-server.tail1234.ts.net` with a valid certificate, and this is the address the app uses. Cloudflare Tunnel works too if you prefer a public address on your own domain (`cloudflared tunnel --url http://127.0.0.1:8796` gives a temporary one for a quick test). With a public address the token is the only protection, so keep it secret and change it if it leaks by deleting `server-token` and restarting the server.

## 3. Install the app on the iPhone

Building an iPhone app needs the full Xcode from the Mac App Store (the Command Line Tools alone are not enough) and XcodeGen, which turns `project.yml` into the Xcode project:

```sh
brew install xcodegen
cd ios
xcodegen generate
open TellusHive.xcodeproj
```

In Xcode, select the TellusHive target, open Signing & Capabilities and choose your team (sign in with your Apple ID under Xcode › Settings › Accounts first). Connect the iPhone with a cable, turn on Developer Mode on the phone when iOS asks (Settings › Privacy & Security › Developer Mode), pick the phone as the run destination and press Run. With a free Apple ID the installed app stops opening after 7 days and has to be run again from Xcode, whereas the Apple Developer Program (US$99 a year) removes that limit and lets you share the app with colleagues through TestFlight.

On first launch, enter the HTTPS address and the token. The app checks that the server answers before it keeps them, and it stores the token in the iPhone's Keychain.

## How the app behaves

The bot list shows every bot on the server with a green dot on the ones that are working. In a chat, the bot's reply streams in as it writes, a status line above the keyboard says what the team is doing now (for example "Designer: calculating B1 beam flexure"), and Stop ends the task. The reports of the team members a bot calls appear as folded notes under the bot's messages, while tool calls and the agents' reasoning stay on the desktop to keep the phone readable. The plus button sends camera shots, photos or files such as PDFs and spreadsheets, and the app shrinks photos to at most 2400 px on the long side before upload. Pictures the team makes show inline, and any other file opens in Quick Look, from where you can share or save it.

The transcript is the same `chat-bot-*.json` file the desktop window keeps, so a conversation started on the phone shows in the Bots workspace and the reverse. Do not run the desktop window on the same data folder while the phone is in a conversation with the same bot, because each keeps its own copy of the open transcript and the last one to save wins. The harness already refuses a second task on a conversation that another TellusHive process is running.

## Limits of this first version

The app does not yet send a push notification when a long task ends while the phone is locked, because that needs an Apple Push Notification service key and a sender on the server. Replies render inline Markdown (bold, italics, code and links) but show tables and headings as plain text. The app covers the Bots workspace only, so teams are reached through a bot that calls them.

## Server API

All requests carry `Authorization: Bearer <token>`. The source is `app/src/server.rs`.

| Method and path | Purpose |
|---|---|
| `GET /api/info` | Server name and version |
| `GET /api/bots` | The bots and which are busy |
| `GET /api/bots/{id}/avatar` | The bot's picture |
| `GET /api/bots/{id}/history` | The conversation so far |
| `POST /api/bots/{id}/messages` | `{"text", "attachments": [path]}`, answered with an event stream |
| `GET /api/bots/{id}/events` | Attach to the running task's event stream |
| `POST /api/bots/{id}/stop` | Stop the running task |
| `POST /api/bots/{id}/new` | Start a new conversation |
| `POST /api/bots/{id}/upload?name=F` | Raw file body, saved in the project's `attachments/` |
| `GET /api/bots/{id}/file?path=P` | A file of the bot's project |
