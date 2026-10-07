# Privacy

This page tells what data Plinth itself collects and sends. It is checklist item G5 in `docs/PRODUCTION.md`. The rule behind it is owner decision 7: **data belongs to the user. It stays on the user's device or in storage that the user owns, never with the vendor.**

## 1. What Plinth collects

Nothing. The Plinth tools, hosts and runtime cores contain no telemetry, no usage statistics, no analytics and no automatic crash reports. Plinth has no server that receives data about users or apps.

If a future version adds crash reports or usage data, it is off by default, the user turns it on (opt in), and this page tells what is sent and where (`docs/EXPERIENCE.md` §4: "Crash reports and usage data: Off").

## 2. Network connections that Plinth makes

Plinth connects to the network only for these reasons, and only to addresses that the user, the app author or the developer chose:

| Connection | When | To |
|---|---|---|
| App update check | An app made with `plinth native` that has `[updates]` in its `plinth.toml` starts | The feed URL of the app author (`docs/UPDATES.md`). The request has no user data and no device id. |
| Hub sources | The user browses, installs or updates apps in the Hub | The registries that the user added (`docs/REGISTRY.md`). |
| An app's own requests | The app calls `plinth:net` | Only hosts that the app declares (`net:<host>`) and the user grants (SPEC §11). |
| Developer tools | A developer runs `npm install`, `npm create plinth` or `cargo` | npm or crates.io. Not part of an installed app. |

## 3. Data of apps

- An app's data (its `store` and `files` spaces) stays on the device, in a separate space for each app (`docs/STORAGE.md` §3). Plinth does not copy it anywhere.
- Plinth gives an app no device id, advertising id, contact list, location or file outside its own space, unless a capability exists for it and the user grants it.
- An app's errors (`error.report`, "This app stopped") go to the local log of the host only.
- When the user removes an app, the host asks if it also deletes the app's data.

## 4. What an app can do

An app is a program of its publisher. With the capabilities that the user grants, an app can send the data that it has to its publisher (for example with `net:*`). The consent screen shows each capability with the publisher's reason. The publisher's own privacy policy covers what it does with that data; Plinth does not.
