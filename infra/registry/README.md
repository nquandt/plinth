# Your personal Plinth registry (plinth.femur.dev)

This folder runs the registry on one machine with Docker. A Cloudflare named tunnel gives it a public HTTPS name. The site has three parts:

- `/` opens the web App Hub (`docs/web-hub.md`). A phone browser can use it.
- `/plinth-registry.json`, `/apps/`, `/packages/`, `/cores/` are the static registry (`docs/REGISTRY.md`).
- `/web/` has the web host files.

The design, the cloud mirror and the plan for a bigger registry are in `docs/REGISTRY-HOSTING.md`.

> **CAUTION:** The registry is public. Do not publish an app that must stay private. Apps hold only code, and the user data stays on the device, but anyone who knows the name can download the packages.

## 1. One-time setup

1. Install Docker, `cloudflared`, and the `plinth` CLI (`cargo install --path crates/plinth-cli --locked`).
2. Make the publisher key: `plinth publisher init --name <your name>`. Put the same name in `publisher` in each app's `plinth.toml`.
3. Log in to Cloudflare: `cloudflared tunnel login`. Choose the zone `femur.dev`. This writes `~/.cloudflared/cert.pem`.
4. Make the tunnel and the DNS name:

   ```sh
   cloudflared tunnel create plinth-registry
   cloudflared tunnel route dns plinth-registry plinth.femur.dev
   cloudflared tunnel token plinth-registry      # prints the token
   ```

5. Copy `.env.example` to `.env`. Put the token in `TUNNEL_TOKEN`. Never commit `.env`.
6. Set `PLINTH_REGISTRY_DIR` in `.env` (or in your shell) to the folder where the registry is built. The default is `./site` for Docker and `~/plinth-registry` for the script, so set one value for both.

## 2. Publish an app

```sh
PLINTH_REGISTRY_DIR=~/plinth-registry bash scripts/registry-publish.sh examples/todo path/to/my-app
```

The script builds and signs each app, builds the Hub app, writes the static registry and copies the web host files. Run it again for each new version. The server needs no restart: it serves the folder.

Add `--sync` to copy the result to the Azure mirror too (`infra/azure/provision.sh`).

## 3. Run it

```sh
cd infra/registry
docker compose up -d
```

Test it:

```sh
curl http://127.0.0.1:8080/plinth-registry.json
curl https://plinth.femur.dev/plinth-registry.json
```

Open `https://plinth.femur.dev/` on a phone. In the Hub, the registry is the site root. On iOS Safari, "Add to Home Screen" makes an app icon.

## 4. Stop or change it

- `docker compose down` stops the server and the tunnel. The name `plinth.femur.dev` then shows a Cloudflare error.
- `cloudflared tunnel delete plinth-registry` removes the tunnel (stop the container first). Remove the DNS record `plinth` in the Cloudflare dashboard.
