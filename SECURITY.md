# Security policy

Plinth runs app code from many publishers on the user's device. A defect that lets an app escape its sandbox, read the data of another app, or use a capability that the user did not grant is a security vulnerability. `docs/THREAT-MODEL.md` tells what Plinth protects and how.

## Report a vulnerability

Do not report a vulnerability in a public issue.

1. Use GitHub private vulnerability reporting: open the repository, select **Security**, then **Report a vulnerability**.
2. Give the Plinth version (`plinth --version`), the host (desktop runner, `plinth native`, web host, Hub), the platform, and the steps to reproduce.
3. If you have a proof of concept, attach the smallest `.plnt` or source app that shows the problem.

We answer in 7 days. We tell you our assessment and the plan for a fix. We ask you to keep the report private for 90 days, or until a fix is released, whichever comes first.

## What we fix

Plinth has rolling support (`docs/PRODUCTION.md` §15). A security fix goes into the newest release.

- A fix in the host (the CLI, the desktop runner, `plinth-host`, the Hub, the web host) goes into the next host release. A desktop app made with `plinth native` gets it through its self-update feed (`docs/UPDATES.md`).
- A fix in a runtime core increments `CORE_PATCH` (SPEC §10.5). Apps with the core policy `follow` or `patch` get the fixed core at the next start. An app with the policy `pinned` keeps its core until its author releases a new version. The Hub tells the user when a pinned core has a known vulnerability (planned).
- Every released runtime core stays available, so an old app keeps running. "Available" does not mean "free of known defects".

## In scope

- An escape from the Wasm sandbox, or a host crash that an app can cause on purpose.
- A host API call that works without the capability, or after the user refused it.
- Access to the data of another app, or to files outside the app's own space.
- A `net` request to the local network without `net.local`.
- A package, a core or an update feed that the host accepts although its digest or signature is wrong.
- In the web host: an app frame that reaches the parent page, another app frame, or the page's storage.

## Out of scope

- A malicious app that does what its granted capabilities allow. For example, an app with `net:*` can send the data that the user types into it. The consent screen tells the user about this risk.
- Denial of service by an app against itself.
- Problems that need a user who has already installed a malicious host binary.

The known limits that are not yet fixed are in `docs/LIMITS.md` and `docs/THREAT-MODEL.md` §6.
