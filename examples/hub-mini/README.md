# Hub mini

A minimal sketch of the Hub UI (`docs/HUB.md` §4.1, §12.2, §15 phase H3).
It imports the privileged `plinth:hub` module and shows the library's apps
in a `List`, with a Launch action per row.

`plinth:hub` needs the `hub.manage` capability (High risk), and the host
grants `hub.manage` only to a package **signed by a key it trusts as a Hub
key**. Every other package that imports `plinth:hub` is refused at load.
That means this example will not just run with `plinth dev` or `plinth
run`: it has to be signed, and the host has to be told to trust that key,
before it does anything useful.

## Running it locally

1. Make a publisher key, if you do not have one yet:

   ```sh
   plinth publisher init --name "Your Name"
   plinth publisher show   # prints "Your Name  ed25519:<key id>"
   ```

2. Edit `plinth.toml` in this folder: change `publisher = "you"` to the
   name `plinth publisher show` printed (the manifest's `publisher` must
   match the signing identity, `docs/HUB.md` §6.1).

3. Build and sign it:

   ```sh
   plinth build examples/hub-mini --sign
   plinth validate examples/hub-mini/dist/hub-mini.plnt
   ```

   `validate` should print `signed by Your Name (ed25519:<key id>)` and
   list `hub.manage` under both `declared` and `reachable`.

4. Tell the host to trust your key, and add the package to a (temporary)
   Hub library:

   ```sh
   export PLINTH_HUB_TRUSTED_KEYS="ed25519:<key id>"   # from step 1
   plinth hub add examples/hub-mini/dist/hub-mini.plnt
   plinth hub run dev.plinth.examples.hub-mini
   ```

   On the first run the consent window asks for `hub.manage` (it is High
   risk, so it is asked every time unless you grant it yourself with
   `plinth hub grants dev.plinth.examples.hub-mini allow hub.manage`, which
   skips the window for scripted testing). With the trusted key set, the
   app opens and lists whatever is already in that `PLINTH_HUB_DIR`
   library (add another example first with `plinth hub add` to see a row).

5. Without `PLINTH_HUB_TRUSTED_KEYS` set to your key (or if the package is
   unsigned), `plinth hub run` refuses it outright, with an error, before
   it opens any window:

   ```
   error: dev.plinth.examples.hub-mini declares `hub.manage` but is signed
   by `ed25519:...`, which this host does not trust as a Hub key (set
   PLINTH_HUB_TRUSTED_KEYS, docs/HUB.md §4.1)
   ```

## What this does not do yet

This is a sketch of the host plumbing, not the Hub UI (`docs/HUB.md` §15,
phase H3 step 2). It has no consent/grant management screen, no block/
unblock buttons, and it reads the library once at startup rather than
after each `launch`/`block`/`unblock`/`setGrant` call. The real Hub UI
will replace it.
