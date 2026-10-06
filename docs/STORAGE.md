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

### 2.1 The profile: your device or your storage, never the vendor's

The host's storage is also the user's **profile**. A profile holds everything that belongs to the user, not to an app:

- the identity: a profile key pair, made on the first device;
- the provider settings (where the user's storage is);
- the Hub library: installed apps, versions, pins, groups;
- every grant, block and policy decision;
- every space, including each app's private space.

**Principle: the data of a Plinth app is on the user's device or in the user's storage, never on the vendor's servers, unless the user gives the app a `net` grant to send it there (S3).**

**A new device.** The user installs a Plinth host (the Hub app, or any Plinth app with the host inside) and connects it to the profile. The host gets the provider settings, restores the library and the grants, and syncs the spaces. Each app then opens with its data, as on the first device, with no account at the vendor.

Design rules for the profile:

1. **End-to-end encryption by default.** The storage provider is not trusted, even when it is the user's own bucket at a cloud company. The host encrypts every object before upload. Keys come from the profile key and never leave the user's devices in plain form. (S11 becomes the default, not an option.)
2. **Device pairing.** A new device joins through an existing device (a QR code or a short code that carries the provider settings and a wrapped key), or through the provider settings plus a recovery phrase. The profile lists its devices, and the user can remove a device; the host then rotates the keys.
3. **Recovery.** A recovery phrase, made at profile creation, restores the profile key if every device is lost. Without it, the data cannot be read. The Hub says this clearly.
4. **More than one profile.** A user can have a personal and a work profile on one device. A profile is a hard boundary: apps, grants and spaces do not cross profiles. An organization can manage a work profile (S10).
5. **Grants travel with the profile.** A grant made on one device is valid on every device of the profile. The consent screen says so.
6. **Offline first.** The device keeps a full local copy of what it uses. Sync runs when the provider can be reached.

This turns the Hub library (docs/HUB.md) and the storage design into one feature. It also gives an answer to "how do I move my apps to a new computer" with no Plinth server in the path.

Open questions for the profile:

- ST-Q7: Which encryption design (for example age or libsodium secretbox for objects, with a key hierarchy: profile key → space keys → object keys)? Encrypted file names, or plain names for easier debugging?
- ST-Q8: Pairing on the web host: a browser profile has no keychain. Keep the key in IndexedDB as a non-extractable WebCrypto key, and require the recovery phrase on each new browser?
- ST-Q9: Can an app ask for the user's identity (for example a public key to sign shared documents) without a vendor account? A `profile.identity` capability, with a separate key for each app so apps cannot track the user across apps.

### 2.2 Instances, hosts and trust (draft)

**Data key.** The data of an app belongs to the triple (profile, app identity, instance). The app identity is (publisher key, app id), as in §3 rule 2.

**Instances.** An instance is one named set of an app's state: its private space, `kv`, secrets, windows and grants. Every app has the instance `default`. A user can make more, for example "Work" and "Personal", to use two accounts of one app at the same time. Two instances of one app cannot read each other's data, except through a space that the user grants to both. The grants of a new instance start as a copy of the app's grants; each instance can narrow them. The Hub shows the instance name with the app name ("Mail — Work"). A native export takes `--instance <name>`; the web Hub puts the instance in the URL.

**Trusted and untrusted hosts.** The host shell sees all data that goes through it. So the data of a profile goes only to hosts that the user trusts:

| Host | Who controls the host shell | Data |
|---|---|---|
| The Hub (desktop, mobile, web) | Plinth, installed by the user | The user's profile (§2.1); synced across the user's devices. |
| A native export (`plinth native`) on a machine with a profile | Plinth's host code inside the export | On that machine, the same per-user store as the Hub, if the package is signed with the same publisher key: it is the same app identity. An unsigned export gets its own store, keyed by its package digest. A native export has no consent screen: it gets its declared capabilities. |
| A web export on a developer's site (SPEC §10.3) | The site owner (the parent page is their code) | Only that site's browser storage. Never the user's profile. |

A later version can offer "Open in my Hub" on a web export: the app then runs in a trusted host with the user's data.

Open questions:

- ST-Q10: Does a native export on a machine without a Hub keep its data in the same place, so that a Hub that is installed later finds it?
- ST-Q11: Instances and `plinth://` links: does a link name an instance, or does the Hub ask?
- ST-Q12: Can an app ask the host to make an instance (for example "Add account"), or only the user?
- ST-Q13: Grants for each instance: is the copy rule right, or should grants stay shared by all instances of an app?

**Manifest.** An app declares the spaces that it wants and why, for example: `spaces = [{ name = "vault", mode = "read-write", reason = "Your notes" }]`. At install or first use, the user picks an existing space or makes a new one for each request. The app cannot name a provider or a location.

## 3. Isolation rules

These rules stop one app from reading or changing the data of another app.

1. **The host derives every location.** The app gives only a space name and a relative path. The host rejects `..`, absolute paths, empty segments, NUL, and encoded forms of these, after Unicode normalization. Then it adds the space's root.
2. **Identity is (publisher key, app id).** The private space of an app is keyed to its publisher key and its id, not to the id alone. Otherwise, another publisher could ship a package with the same id. An unsigned app gets a key from its package digest, and the Hub warns about it.
3. **Defense in depth with scoped credentials.** Where the provider supports it, the host makes a credential for each grant that works only on that grant's location: an S3 STS session policy on the prefix, or an Azure SAS token on the container or directory. Then a defect in the host's path checks cannot cross partitions. Where the provider does not support it, use one bucket or container for each space.
4. **No data crosses between apps without an explicit grant.** Data moves between apps only in three ways: a grant to a shared space (S5), a one-time share that the user starts (S8), or a cross-app data capability (§3.1). In each case, the user approves it. There is no default access, and two apps from the same publisher also get no default access.
5. **A grant is visible and can be removed.** The Hub lists every grant. When the user removes a grant, the host closes the app's handles. When the user removes an app, the host asks if it should also delete the app's private space (S13).
6. **Denied calls never trap.** As for every host API, a denied call returns an error value.

### 3.1 Cross-app data capability

One registered app (the **reader**) can ask for access to the data of another registered app (the **owner**). The host allows it only if all of these conditions are true:

1. **The owner exports the data.** The owner's manifest declares what it lets other apps read, for example `exports = [{ name = "notes", space = "private", path = "notes/", mode = "read", reason = "Your notes as Markdown files" }]`. Data that the owner does not export can never be read by another app.
2. **The reader asks by full identity.** The reader's manifest declares a capability such as `data.read:<owner publisher key>/<owner app id>/notes`, with a reason. The app id alone is not enough, so a package with the same id from another publisher cannot take the place of the owner.
3. **The user approves it.** The consent screen names both apps, both publishers and the exported data. The Hub shows the grant on both apps' pages, and the user can remove it at any time.
4. **Read is the default.** Write access needs `mode = "read-write"` in the owner's export and a separate approval at a higher risk level.
5. **The host does every access.** The reader gets a handle to the exported path only, with the same path checks and scoped credentials as any grant. The reader never gets the owner's root or the owner's other data.
6. **The grant follows identity changes.** If the owner's publisher key changes (a key rotation or a different publisher), or the owner removes the export in an update, the host stops the grant until the user approves it again.

The owner learns, through an event, that a reader has a grant. The owner cannot see what the reader reads.

**Status: deferred.** §3.2 lists the risks. The first slice (§6) does not build this capability. It uses only user-owned spaces and one-time shares.

### 3.2 Risks of cross-app access, and mitigations

Even with the conditions in §3.1, a lasting grant between apps is risky:

| Risk | Example | Mitigation |
|---|---|---|
| Exfiltration | The reader also has `net:*` and sends the notes to its server. | The consent screen shows the combination ("can read your notes AND send data to the internet") at the highest risk level. A host policy can refuse the combination. |
| Consent fatigue and social engineering | An app asks for access with a false reason, and the user approves without reading. | Prefer one-time shares (S8), where the user picks each item. A lasting grant is an advanced setting, not a normal prompt. |
| Exports that are too broad | The owner exports its whole space by mistake. | An export names one path and one mode. The Hub shows the size and the number of files in an export. |
| Chains | App A grants to B, and B gives the data to C. | A reader cannot export data that it got through a grant. The host can enforce this at the handle level (granted handles cannot be re-exported); it cannot stop a reader that copies the data into its own private space. Treat this as a residual risk, and show it. |
| A confused deputy | The reader tricks the owner into a write through the owner's own UI or links. | Read is the default. Write needs a separate approval. Links (`plinth://`) never carry data access. |
| Silent long-term access | A grant that the user forgot. | Grants expire (for example after 30 days without use) and the Hub shows the last access. An access log in the Hub. |
| Identity changes | A new owner key after a sale of the app. | §3.1 rule 6: the grant stops until the user approves it again. |

The exfiltration and consent risks also apply to a grant to a shared space (S5): an app with a grant to the user's vault and with network access can send the vault out. So the "data plus network" warning applies to every grant, not only to §3.1.

The safest default is: **no lasting access between apps.** Data moves only through spaces that the user owns and grants (S5) or through one-time shares (S8). Build the lasting capability only if a real app needs it, and only with the mitigations above.

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
| S8 | A share action that the user starts. The host copies the data into the target app's private space or gives a one-time token. For lasting access, the cross-app data capability (§3.1). |
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
6. Not in this slice: the cross-app data capability (§3.1, deferred). A second app tests the walls instead: without a grant it cannot read the notes app's private space or the vault; a package with the same id from another publisher gets nothing; removing a space grant closes the handle.

The slice does not build the full profile (§2.1). It keeps the profile in mind: object layout, keys and the library format must not block it later. Encryption of synced objects is in the slice if ST-Q7 has an answer by then.

Editor: Markdown source in a `TextArea`, with a new `Markdown` display control for the reading view (headings, lists, links, `[[wiki links]]`, code). A rich-text control is a later decision.
