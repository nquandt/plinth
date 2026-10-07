# Guide: publish an app

This guide tells you how to sign your app and put it in a registry, so that users can install it with the Plinth Hub. It is for app authors. The details are in `docs/HUB.md` §6 and §8, `docs/REGISTRY.md` and `docs/web-hub.md`.

There is no `plinth publish` command yet (`docs/PRODUCTION.md` T6). You make a static registry folder and put it on a web server.

## 1. Make a publisher key

Do this one time on your computer:

```sh
plinth publisher init --name "example"
plinth publisher show          # prints the name and the key id, ed25519:...
```

- The name must be equal to `publisher` in the `plinth.toml` of each app that you sign. If not, `plinth build --sign` stops with "the manifest's publisher (...) does not match the signing identity (...)".
- The key is in `%APPDATA%\plinth\publisher` on Windows (`$XDG_CONFIG_HOME/plinth/publisher` on other systems; `PLINTH_PUBLISHER_DIR` changes it). The file `key.ed25519` is your private key. Keep a backup in a safe place, and never put it in a repository.
- Use the same key for all versions of an app. A registry and the Hub refuse a new version that a different key signed. Key rotation is not built yet: a new key needs a new app id.

## 2. Sign the package

```sh
plinth build --sign                 # build and sign
plinth sign dist/<name>.plnt        # or: sign a package that you built before
plinth validate dist/<name>.plnt    # prints "signed by <publisher> (<key id>)"
```

The signature (`signature.json` in the `.plnt`) covers the digest of each file in the package. A package whose signature does not check out is refused everywhere. It is never treated as unsigned.

An unsigned package still works, with a warning. But on the desktop, an unsigned package gets a new private file space for each build (see [storage.md](storage.md) §7), and the Hub cannot show who made it. Sign each package that you give to users.

## 3. Make a registry

A registry is a folder of static files. Any static web server, a CDN or a local folder can serve it.

1. Put your `.plnt` files in a folder (or in `<folder>/incoming/`).
2. Run:

   ```sh
   plinth registry build <folder> --with-core
   ```

   The command writes `plinth-registry.json`, `apps/index.json`, `apps/<id>/index.json` and `packages/<sha256>.plnt`. `--with-core` also copies the runtime core of the `plinth` binary to `cores/`, so that a Hub can install it.

3. Put the folder on a web server (`https://`). `http://` is allowed only for `localhost` and `127.0.0.1`.

To add a version, put the new `.plnt` in the folder and run `plinth registry build` again. The command adds the version. It never changes a version that is already in the registry. A second run with the same input gives the same output.

`plinth registry build` refuses a package when:

- it is not a valid `.plnt`;
- it does not link against a core of the `plinth` binary under its `core` policy (see [updates.md](updates.md) §2);
- its signature does not check out;
- it can reach a capability that `plinth.toml` does not declare;
- another package with the same id and version, but different bytes, is already registered;
- a different key signed the previous version of the same app.

## 4. Test the registry on your computer

```sh
plinth registry serve <folder>              # http://127.0.0.1:8080/
plinth registry serve <folder> --port 9000  # another port
plinth registry serve <folder> --web        # also the web host files: the web App Hub
```

The server listens on `127.0.0.1` only. Then use the registry from the desktop Hub:

```sh
plinth hub source add mine http://127.0.0.1:8080
plinth hub search <text>
plinth hub install <app id>
```

The web App Hub (`--web`) needs a signed Hub app in the registry and `--hub-trusted-key` on `plinth registry build`. `docs/web-hub.md` §1 tells how. `bash scripts/web-hub-demo.sh` makes a demo registry with the example apps.

## 5. What the Hub checks

Before an app runs, the Hub and the host check it again. They do not trust the registry for the content.

| Check | Desktop Hub | Web App Hub |
|---|---|---|
| SHA-256 of the package against the registry | Yes | Yes (`docs/web-hub.md` §4.3) |
| The manifest: id, version, the digest of the app module | Yes | Yes |
| The Ed25519 signature | Yes. A bad signature is refused. | Yes, with WebCrypto. A browser without Ed25519 gives "not checked"; the app still runs. |
| Same key as the earlier version | Yes | Yes, on an update |
| A blocked app or a blocked publisher key | Refused | Refused |
| The capabilities that the app module can reach are declared | Yes. A package that can reach an undeclared capability does not run. | No. The host checks each call against the declared capabilities when the app runs. |
| The digest of the core | Yes | Yes |

Then the host shows the consent window for the capabilities that need a decision (`docs/HUB.md` §7.2): None and Low risk capabilities are allowed with no question; Medium and High risk capabilities wait for the user. An update that adds a capability asks the user again before the new version runs. If the user cancels, the previous version runs.

## 6. Before you publish

- Give each capability a clear `rationale`. The consent window shows it next to the risk level.
- Ask only for what you use. `plinth validate` shows "declared" and "reachable" capabilities; a capability that is declared but not reachable is shown, and the Hub does not ask for it.
- Prefer `net:<host>` to `net:*` (see [network.md](network.md)).
- Check accessibility (see [accessibility.md](accessibility.md)).
- Change `version` in `plinth.toml` for each release. A registry keeps each version for ever.
