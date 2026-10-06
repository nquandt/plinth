# Plinth Hub UI

The Hub UI (`docs/HUB.md` §4.1, §15 phase H3) as a Plinth app. It uses the
privileged `plinth:hub` module (core 1.9) and has three screens:

- **Library**: every app in the library, with group tabs, a text filter,
  and the highest risk of each app. The **New group** action makes a group.
  When the Hub starts, it checks the source of each app for a newer
  version (`docs/HUB.md` §9.2). A badge shows the number of updates, and
  the row of each app with an update shows "Update available".
  **Check for updates** checks again.
- **App** (push it from a library row): the publisher and the signature,
  the capability label (`docs/HUB.md` §7.2) with a risk level, the reason
  of the app and a toggle for each grant, the text "This app cannot …",
  the groups of the app, and **Open**, **Block**/**Unblock** and
  **Remove from library**. If the app is signed, **Block publisher**
  blocks every app of that publisher key. If an update is available,
  **Update to <version>** installs it and shows the capabilities that it
  adds; the Hub asks for them before the new version opens. The
  **Versions** list shows the installed versions: select one to pin the
  app to it, and **Run the newest version** removes the pin.
- **Discover**: a search across every configured source
  (`plinth hub source add`). Select a result to install it, or to show it
  if it is in the library.

**Open** asks the host to launch the app. The host opens it in a new
window, with the consent window first if the app has capabilities that
are not decided (`docs/HUB.md` §4.2, §7.3).

## Trust

`plinth:hub` needs the `hub.manage` capability (High risk). The host gives
`hub.manage` only to a package that a **trusted Hub key** signed. The host
refuses every other package that declares `hub.manage`, before it runs.

## Run it

1. Make a publisher key, if you do not have one:

   ```sh
   plinth publisher init --name "Your Name"
   plinth publisher show   # prints "Your Name  ed25519:<key id>"
   ```

2. In `plinth.toml`, set `publisher` to the name that `plinth publisher
   show` printed (`docs/HUB.md` §6.1).

3. Build and sign the app:

   ```sh
   plinth build examples/hub --sign
   plinth validate examples/hub/dist/hub.plnt
   ```

4. Trust your key, add the Hub UI and some apps to the library, then run
   it:

   ```sh
   export PLINTH_HUB_TRUSTED_KEYS="ed25519:<key id>"
   plinth hub add examples/hub/dist/hub.plnt
   plinth hub grants dev.plinth.hub allow hub.manage   # or answer the consent window
   plinth hub add examples/notes/dist/notes.plnt
   plinth hub ui
   ```

   `plinth hub ui [<app id>]` runs the Hub UI app (`dev.plinth.hub` if you
   do not give an id). `plinth hub run dev.plinth.hub` does the same.

The Hub window stays open when you open apps. Each app gets its own
window. The process stops when you close the last window.

To try **Discover**, add a source first, for example a static registry
that `plinth registry build` made: `plinth hub source add local <folder>`.

## In a browser

The same package runs in the web App Hub (`docs/web-hub.md`):
`bash scripts/web-hub-demo.sh` signs it with a throwaway demo key and
serves it. On the web the library is the registry listing (browse mode),
and the host opens each app in a sandboxed frame after its consent window.
