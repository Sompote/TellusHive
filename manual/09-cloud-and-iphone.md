# 9. The cloud server and the iPhone app

TellusHive can run its bots on a cloud machine without the window, so you can reach them from a phone while the work goes on in the cloud. The server is the `serve` mode of the normal `tellushive` program, and an iPhone app talks to it. This chapter gives the outline, and [ios/README.md](../ios/README.md) gives every step.

The iPhone app is a preview of the source code. It has not yet been built or tested on a device, so treat this chapter as a guide for developers rather than a finished feature.

## The server

On the cloud machine, build TellusHive with `cargo build --release` and copy your data folder to it, so the server has your provider keys, bots, teams and standards. Then start the server:

```sh
./target/release/tellushive serve            # listens on 127.0.0.1:8796
```

On its first start the server writes a random token to the file `server-token` in the data folder and prints it, and every request from the phone must carry that token. You can also set it with `--token` or the environment variable `TELLUSHIVE_SERVER_TOKEN`. A task runs on the server whether or not the phone stays connected, so you can lock the phone during a long design and the app picks up the conversation again when you return. The transcript is the same file the window keeps, so a conversation started on the phone continues in the Bots workspace and the reverse. Do not run the window on the same data folder while the phone is in a conversation with the same bot, because each keeps its own copy of the open transcript and the last one to save wins.

## A safe address

The server listens on `127.0.0.1` because it speaks plain HTTP, and the token must cross the internet only inside HTTPS. Tailscale is the simplest choice, because the server then stays off the public internet: install Tailscale on the server and on the phone, sign both into one account, and run `sudo tailscale serve --bg 8796` on the server, which prints an HTTPS address such as `https://tellus-server.tail1234.ts.net`. Cloudflare Tunnel works too if you prefer a public address, and then the token is the only protection.

## The iPhone app

The app shows your bots like a messenger, streams each reply as the bot writes, shows what the team is doing with a **Stop** button, sends photos from the camera or library and files such as PDFs, and opens the calculations, drawings and reports in Quick Look. Building it needs the full Xcode, XcodeGen and an Apple ID, as described in [ios/README.md](../ios/README.md). On first launch, enter the HTTPS address and the token, which the app keeps in the iPhone's Keychain.
