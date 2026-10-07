# Guide: use the network

This guide tells you how to make HTTP requests from a Plinth app. It is for app authors. The reference is [host-apis.md](../host-apis.md) (`plinth:net`). The rules come from `SPEC.md` §11 and `docs/THREAT-MODEL.md`.

## 1. What you get

`plinth:net` has one function, `fetch`. It sends one HTTP request and gives you the status and the body as text. There is no WebSocket. Only text bodies (UTF-8) are supported.

## 2. Declare the hosts

An app can send a request only to a host that `plinth.toml` names. Declare one capability for each host:

```toml
[[capabilities]]
name = "net:api.example.com"
rationale = "Load quotes and send your notes."
```

| Capability | Lets the app reach | Risk level on the consent screen |
|---|---|---|
| `net:<host>` | That host only, an exact match of the host name (for example `net:api.example.com`). | Medium |
| `net:*` | Any public host. Use it only when you must, and tell the reason in `rationale`. | High |
| `net.local` | Private addresses: your computer and devices on your local network (§4). | Medium |

If your code calls `fetch` and `plinth.toml` declares no `net:` capability and no `net.local`, `plinth check` gives the error `PL1007`. The compiler cannot know the host of each request, because the URL is a string at run time. Thus the host checks each request again when it runs.

## 3. Example

This example uses the `await` form and the callback form. It is checked with `plinth check` against `std/net.d.ts`.

```tsx
import { app, signal, Screen, Section, Text, Button } from "plinth:ui";
import { fetch } from "plinth:net";
import { JSON } from "plinth:core";

interface Quote {
  content: string;
  author: string;
}

const shown = signal("Press Load.");
const busy = signal(false);

async function load(): Promise<void> {
  busy.set(true);
  const r = await fetch("https://api.example.com/quote", null);
  busy.set(false);
  if (r.error !== null) {
    // Denied ("denied:undeclared", "denied:refused") or a network error ("network: ...").
    shown.set("Could not load: " + r.error);
    return;
  }
  if (!r.ok) {
    // The server answered with a status that is not 2xx.
    shown.set("The server answered " + r.status + ".");
    return;
  }
  const q = JSON.parse<Quote>(r.text);
  shown.set(q === null ? "The answer is not a quote." : q.content + " (" + q.author + ")");
}

function send(text: string): void {
  // The callback form. A POST with a text body.
  fetch("https://api.example.com/notes", { method: "POST", body: text }, (r) => {
    shown.set(r.ok ? "Sent." : "Not sent: " + (r.error ?? "HTTP " + r.status));
  });
}

function Home() {
  return (
    <Screen title="Network">
      <Section>
        <Text>{shown()}</Text>
        <Button label={busy() ? "Loading…" : "Load"} onPress={() => { load(); }} />
        <Button label="Send" onPress={() => send("hello")} />
      </Section>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Network", component: Home } } });
```

`examples/quotes` is a complete app with `fetch`.

`options` is `null` for a plain `GET`, or an object literal with `method` (`"GET"`, `"POST"`, `"PUT"`, `"PATCH"` or `"DELETE"`), `headers` (a `Map<string, string>`) and `body` (a string). The compiler accepts only an object literal here, not a variable.

## 4. What "private" means

A request to a private address needs `net.local`. A `net:<host>` capability does not give access to a private address, also if the name matches. "Private" means every address that is not a public unicast address:

- `localhost`, names that end with `.localhost` or `.local`;
- loopback (`127.0.0.0/8`, `::1`), private ranges (`10/8`, `172.16/12`, `192.168/16`), shared (CGNAT, `100.64.0.0/10`), link-local (with the cloud metadata address `169.254.169.254`), multicast, reserved and broadcast addresses;
- IPv6 unique-local (`fc00::/7`), link-local and site-local addresses;
- an IPv4 address in an IPv6 form (for example `::ffff:127.0.0.1`) that is one of the above.

The desktop host checks the host name in the URL, and it checks the addresses again after the DNS lookup. Thus a public name that resolves to a private address is also denied without `net.local`. The web host can check only the host name in the URL (`docs/THREAT-MODEL.md` §6 item 1).

## 5. The result

`fetch` never throws, and its promise never rejects. You always get a `Response`:

| Field | Value |
|---|---|
| `ok` | `true` only for a 2xx status with no error. |
| `status` | The HTTP status, or `0` if the request did not reach the server. |
| `text` | The body as text, or `""` on an error. |
| `error` | `null` if the server answered (also for a status that is not 2xx). Else the reason (below). |

| `error` | Cause |
|---|---|
| `"denied:undeclared"` | `plinth.toml` does not declare a capability for this host. On the desktop, also a public name that resolves only to private addresses. |
| `"denied:refused"` | The user refused the capability. |
| `"denied:unsupported"` | The host does not support the request, for example a URL with no host. |
| `"network: <detail>"` | The request failed: no connection, the time limit, a body that is too large, or (on the desktop) a body that is not UTF-8. |

Thus check `error` first, then `ok` (as the example does). A status that is not 2xx has `ok === false` and `error === null`.

## 6. Limits and differences between hosts

| Item | Desktop | Web |
|---|---|---|
| Time limit | 20 s for each request. | 20 s for each request. |
| Body limit | 8 MiB. A larger body gives a `"network: ..."` error. | The host code has no limit. |
| Redirects | The host does not follow a redirect. | The host does not follow a redirect (`redirect: "manual"`). |
| Cookies | Not checked. | The request has no cookies (`credentials: "omit"`). |
| Origin | — | In a sandboxed frame (the web Hub, a web export), the page makes the request, so the request has the origin of the page. The server must allow CORS for that origin. |
| Private check | The URL host and the DNS result. | The URL host only. |

What `fetch` returns for a redirect response (a 3xx status) is not checked on either host. Do not use a URL that redirects.

## 7. Consent

The user sees each declared host on the consent screen ("connect to api.example.com"). A granted `net:*` lets the app send its data to any host, so the consent screen shows it as High risk. A user can refuse a capability; then each request answers `"denied:refused"`. Show this to the user as a normal state, for example "Network access is off for this app".

A web export (`plinth build --target web`) has no consent screen: the app gets all the capabilities that `plinth.toml` declares.
