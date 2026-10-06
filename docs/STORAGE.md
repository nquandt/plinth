# Storage design (draft)

Status: draft for discussion (2026-10-06). Nothing in this document is built yet, except `store.kv` and `fs.pick` in the SPEC.

This document gives one model for app data in Plinth. The model must cover many scenarios: private app data, sync across devices, the app vendor's cloud, storage that the user chooses, folders that other tools also use, shared team storage, and more. §1 lists the scenarios. §2 gives the model. §3 gives the rules that keep apps apart. §4 shows how each scenario maps to the model. §6 is the first slice to build.

## 1. Scenarios

| # | Scenario | Example |
|---|---|---|
| S1 | Private app data on one device | Settings, a cache, a draft |
| S2 | Private app data, synced across the user's devices | Notes on a laptop and a phone |
| S3 | The app vendor's own cloud service | A mail app that talks to its mail server |
| S4 | Storage that the user chooses (bring your own) | The user's S3 bucket, Azure Blob, or a NAS |
| S5 | One data set that several apps use | A notes vault that a notes app and a search app both open |
| S6 | A folder that tools outside Plinth also use | An Obsidian vault folder on disk, synced by another tool |
| S7 | A file the user picks one time | Open or save one document |
| S8 | Share data from one app to another, one time | "Send this note to the mail app" |
| S9 | A space shared by several people | A team bucket |
| S10 | Storage that an organization enforces | A company forces its own backend and a policy |
| S11 | End-to-end encryption | The storage provider cannot read the data |
| S12 | Large media | Range reads and streaming of video |
| S13 | Quotas, export, backup, and removal on uninstall | "Remove this app and its data" |
| S14 | Different platforms | Desktop files, browser storage (OPFS, IndexedDB), mobile sandboxes, iCloud and Google Drive |

## 2. Model

The model has six concepts. The host owns all of them. An app never sees a credential, a bucket, an endpoint or an absolute path.

1. **Provider.** A backend implementation in the host: the local file system, browser storage (OPFS), S3-compatible storage (AWS, rustfs, MinIO), Azure Blob, and later WebDAV, Dropbox, Google Drive or iCloud. The set of providers is open. A provider can be built into the host, or added later as a host plugin.
2. **Space.** A named storage root that **belongs to the user**, not to an app. A space binds one provider and one location (a folder, a bucket and a prefix, or a container) to a set of policies: sync, encryption, quota, read-only, and members (S9). Examples: "Private data of Notes", "My vault", "Team docs".
3. **Grant.** Access for one app to one space, with a mode (read, read and write) and an optional sub-path. The Hub shows grants like capabilities, and the user can remove them. Every app has one private space that it always gets; the host makes it at install time.
4. **File API.** One app-facing API over spaces (working name `plinth:files`). The app opens a space by the name in its manifest (for example `private` or `vault`), then uses relative paths: `list`, `read`, `write`, `stat`, `remove`, `watch`. The same app code works for every provider.
5. **Sync engine.** A host service that copies a space between a local copy and a remote provider. The app works offline against the local copy. The host finds conflicts with ETags or version ids, keeps both versions, and tells the app through an event. The app does not need network access or credentials for sync.
6. **Secrets.** A small `plinth:secrets` API that keeps an app's own tokens (S3), encrypted and separate for each app, in the OS keychain where possible. It is not for spaces; spaces need no app secrets.

**Manifest.** An app declares the spaces that it wants and why, for example: `spaces = [{ name = "vault", mode = "read-write", reason = "Your notes" }]`. At install or first use, the user picks an existing space or makes a new one for each request. The app cannot name a provider or a location.

## 3. Isolation rules

These rules stop one app from reading or changing the data of another app.

1. **The host derives every location.** The app gives only a space name and a relative path. The host rejects `..`, absolute paths, empty segments, NUL, and encoded forms of these, after Unicode normalization. Then it adds the space's root.
2. **Identity is (publisher key, app id).** The private space of an app is keyed to its publisher key and its id, not to the id alone. Otherwise, another publisher could ship a package with the same id. An unsigned app gets a key from its package digest, and the Hub warns about it.
3. **Defense in depth with scoped credentials.** Where the provider supports it, the host makes a credential for each grant that works only on that grant's location: an S3 STS session policy on the prefix, or an Azure SAS token on the container or directory. Then a defect in the host's path checks cannot cross partitions. Where the provider does not support it, use one bucket or container for each space.
4. **Only the user shares.** Data moves between apps only through a grant to a shared space (S5), or through a one-time share that the user starts (S8). An app cannot ask the host for another app's space by name.
5. **A grant is visible and can be removed.** The Hub lists every grant. When the user removes a grant, the host closes the app's handles. When the user removes an app, the host asks if it should also delete the app's private space (S13).
6. **Denied calls never trap.** As for every host API, a denied call returns an error value.

## 4. Scenarios mapped to the model

| Scenario | How the model covers it |
|---|---|
| S1 | The private space on the local provider. |
| S2 | The private space with sync to a remote provider that the user configured. |
| S3 | Outside spaces: `net:<vendor host>` and `plinth:secrets`. |
| S4 | The user configures a provider one time in the host, then puts spaces on it. |
| S5 | One user space with grants to several apps. |
| S6 | A space on the local provider that points to an existing folder. Other tools can change the files, so `watch` events are necessary. |
| S7 | `fs.pick` (SPEC.md §11): the user picks a file, and the app gets a token for that file only. |
| S8 | A share action that the user starts. The host copies the data into the target app's private space or gives a one-time token. |
| S9 | A space with members. The provider's access control is the source of truth. This needs more design. |
| S10 | A host policy that limits which providers can hold spaces, like the Hub's global policy. |
| S11 | A space policy: the host encrypts before the data leaves the device and keeps the key in the keychain. |
| S12 | Range reads in the file API, and streams in a later version. |
| S13 | Quotas as a space policy; export of a space as a zip; delete on uninstall. |
| S14 | A provider for each platform behind the same file API: OPFS in the browser, the app sandbox on mobile. |

## 5. Open questions

- ST-Q1: Does the file API also give a key-value or database view (`store.sql`), or does an app build that on files?
- ST-Q2: How does the web host reach S3 or Azure? The bucket needs CORS, or the hub server must act as a proxy. Where does the web host keep the credentials?
- ST-Q3: Conflict policy: keep both versions and tell the app (the draft), or let the app merge (CRDTs, later)?
- ST-Q4: Can a provider be a privileged Plinth package (like the Hub app), or only native host code?
- ST-Q5: Does rustfs support STS session policies? If not, the local tests use one bucket for each space.
- ST-Q6: Space members (S9): who manages them, and how does the host show them?

## 6. First slice: the notes app (VALIDATION.md V2)

The Obsidian-like notes app is the first user of this design. The slice builds only what it needs:

1. `plinth:files` with `list`, `read`, `write`, `stat`, `remove`, `watch`, on the private space and one granted space named `vault`.
2. Providers: the local file system (desktop), OPFS (web), and an existing folder as a space (S6), so that the app can open a real Obsidian vault.
3. The sync engine for one space, with two remote providers: S3-compatible (tested against rustfs in Docker) and Azure Blob (tested against Azurite in Docker). Conflicts keep both versions.
4. The Hub: configure a provider, make a space, grant it, remove a grant. The consent screen shows the space requests.
5. Isolation tests: a second app cannot reach the vault without a grant; path tricks fail; scoped credentials where rustfs and Azurite support them.

Editor: Markdown source in a `TextArea`, with a new `Markdown` display control for the reading view (headings, lists, links, `[[wiki links]]`, code). A rich-text control is a later decision.
