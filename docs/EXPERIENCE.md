# User experience: scenarios, defaults and principles (draft)

Status: draft for discussion (2026-10-06). This document describes how Plinth should feel to the people who use it, at large scale. It connects the technical drafts: `docs/HUB.md` (the Hub, consent, registry), `docs/STORAGE.md` (profiles, spaces, instances, sync) and SPEC.md §10.3 (web export).

The goal: **privacy and security that are on by default and that the user does not have to manage.** A user should never need to understand keys, origins, sandboxes or capabilities to be safe.

## 1. Principles

1. **Safe without questions.** The safe choice is the default. Low-risk permissions are given without a prompt (HUB.md risk levels). The user sees a prompt only when a choice changes what an app can do with their data.
2. **Ask at the moment of use, in plain words.** A prompt comes when the app first needs a permission, with the app's own reason, not as a list at install time. One decision for each prompt.
3. **No accounts to start.** Plinth works on the first device with no sign-up. The profile is made in the background. Sync, other devices and backups are optional and come later.
4. **Your device or your storage, never the vendor's** (STORAGE.md §2.1). End-to-end encryption is always on for synced data. The user does not choose it, and cannot turn it off by mistake.
5. **Every choice can be seen and undone.** One page shows what each app can do and what it stores, with the last use. Every permission can be removed. Every app can be removed with its data.
6. **Honest words.** The UI says "unknown publisher", "can send data to the internet", "only you can recover this". It does not hide risk, and it does not frighten without a reason.
7. **Simple first, advanced hidden.** S3 buckets, keys, instances and policies exist, but they are in "Advanced". The normal path never shows them.
8. **Offline first.** Every app and the Hub work without a network. Sync runs when it can.

## 2. Words in the UI

The documents use technical words. The UI uses these words instead:

| Technical word | Word in the UI |
|---|---|
| capability, grant | permission |
| consent screen | "Allow <app> to …?" |
| space | storage, or a named place such as "My notes vault" |
| private space | "data of <app>" |
| instance | account (as in "Add another account") |
| profile | "your Plinth" or the profile name ("Personal", "Work") |
| provider | "where your data is kept" |
| publisher key | publisher; "verified" or "unknown" |
| registry, source | store, catalog |
| browse mode | "Try" |
| install, library | "Keep", "Your apps" |

## 3. Ideal scenarios

Each scenario lists the steps that a person takes. Fewer steps is better. A step marked "(auto)" needs no action.

### A. Try an app from a link (target: under 5 seconds, no questions)

1. The person opens a link to an app (a web page, a chat message, a QR code).
2. The app opens: in the Hub if it is installed, otherwise in the browser (web Hub). (auto)
3. A small bar shows the app name, the publisher (verified or unknown) and a one-line summary of what the app can do ("Saves on this device · Talks to api.example.com"). (auto)
4. The app asks for a permission only when it needs it.

A tried app keeps its data only for the session, unless the person chooses "Keep".

### B. Keep an app (one tap)

1. The person taps "Keep".
2. The app is in "Your apps" on every device of the profile, with its data. (auto)

### C. A new device (target: under 2 minutes, no passwords)

1. The person installs the Hub on the new device.
2. The Hub asks: "Use Plinth on another device too?" The person opens the Hub on an existing device and scans a QR code (or types a short code).
3. Apps, permissions and data come back, encrypted on the way. (auto)

### D. All devices lost

1. The person installs the Hub and chooses "Recover".
2. They type the recovery phrase, or use the recovery file, from the "recovery kit".
3. The Hub gets the data from where it was kept and decrypts it. (auto)

The Hub makes the recovery kit when sync is first turned on. It offers to print it or save it, and reminds the person one time later. It says plainly: "Without this, nobody can recover your data, not even Plinth." This is the one place where security has a cost for the user. It must be clear, short and done one time.

### E. Choose where data is kept

1. Settings → "Where your data is kept".
2. The simple choices come first:
   - "This device only" (the default);
   - "A folder that is synced already" (OneDrive, iCloud Drive, Dropbox, Google Drive, a NAS). Plinth writes encrypted files to the folder, and the existing sync tool copies them. No new account is needed, and most people already have such a folder.
3. "Advanced": S3-compatible storage, Azure Blob, WebDAV, with fields for the endpoint and the credentials.

### F. Two accounts of one app

1. In the app's menu or on its page in the Hub: "Add another account".
2. The person names it ("Work"). A new, empty copy of the app opens. (auto)
3. Both show in "Your apps" as "Mail — Personal" and "Mail — Work". They cannot see each other's data.

### G. "What can my apps do?"

1. Settings → "Privacy".
2. One list: each app, its permissions, the last time it used each one, and how much data it stores.
3. Each line has "Remove". Apps that read data AND can send data to the internet are at the top, marked.
4. Permissions that an app has not used for 90 days are removed automatically, with a notice. (auto)

### H. Remove an app

1. "Remove" on the app page.
2. The Hub asks one question: "Also delete its data on all your devices?" with "Keep data" as a choice for a later return.

### I. Share with another person (later)

1. "Share" on a notes vault (a space).
2. The Hub makes an invite link. The other person opens it in their Hub and accepts.
3. Both profiles see the vault. Each person's Hub encrypts for both. The owner can remove the other person at any time.

### J. A developer ships an app

1. `npm create plinth` → `npm run dev` → the app runs with hot reload.
2. `plinth build`. One `.plnt` for every host.
3. Choose one or more ways to ship: a catalog (`plinth publish`), a web page (`--target web`), a single HTML file, or a desktop program (`plinth native`).
4. `plinth publisher init` one time; packages are signed automatically after that. A publisher can verify a domain (a DNS TXT record or a `/.well-known/plinth` file) to show "verified: example.com" in every Hub.

### K. Work and organizations

1. The organization gives a link or a QR code: "Join the Work profile of Example Corp".
2. The Hub adds a second profile, "Work", next to "Personal". (auto)
3. The organization's policy sets where Work data is kept, which apps are allowed, and which permissions are blocked. The person sees the policy on one page.
4. Work and Personal never share apps, permissions or data. The organization cannot see the Personal profile.

### L. A family or a child

1. A parent adds a profile for a child, or uses the device's user accounts.
2. The child's profile has an allow-list: only apps that the parent approved, and no high-risk permissions without the parent's approval.

## 4. Defaults

| Area | Default | Who can change it |
|---|---|---|
| Low-risk permissions (save on this device) | Allowed, no prompt | The user, on the Privacy page |
| Medium-risk permissions (network to one named server, clipboard read) | Prompt at first use | The user |
| High-risk permissions (network to any server, Hub management, data of another app) | Prompt at first use with a warning; refused if a policy says so | The user, the organization |
| "Reads your data AND sends to the internet" | Marked at the top of the Privacy page | — |
| Unused permissions | Removed after 90 days, with a notice | The user (can turn off) |
| Updates with no new permissions | Automatic | The user (can pin a version) |
| Updates with new permissions | Wait for the user's approval; the old version keeps working | — |
| Unknown publisher | Allowed to run, marked "unknown" | The user, the organization (can block) |
| Data location | This device only | The user |
| Encryption of synced data | Always on | Nobody |
| Crash reports and usage data | Off | The user (opt in) |
| Tried apps | Data for the session only | "Keep" |

## 5. Security that the user does not see

These protections are always on and need no decision:

- Every package is checked against its digest. A signed package is checked against its publisher key (HUB.md H1).
- Every app runs in a sandbox: Wasm with a time and memory limit on the desktop and mobile, a sandboxed iframe on the web (SPEC.md §10.3).
- An app can only use permissions that its package declares, and the host proves this from the app's imports before it runs.
- Apps cannot see each other's data (STORAGE.md §3).
- Synced data is encrypted on the device before it leaves.
- A web page that embeds a Plinth app never gets the user's profile data (STORAGE.md §2.2).

## 6. What can go wrong, and the answer

| Problem | Answer |
|---|---|
| Too many prompts, so people approve without reading | Low risk needs no prompt; prompts only at first use; one decision each; the 90-day expiry removes old grants. |
| People lose the recovery kit | Device pairing covers most cases (any one remaining device can add new ones). A reminder after setup. The cost is stated plainly. |
| A malicious app with a good reason text | Publisher verification, catalog review, the "reads AND sends" mark, and a block list that the Hub updates. |
| A fake copy of a known app | The app identity includes the publisher key. A copy from another key is a different app with an "unknown" or different publisher. |
| Sync conflicts | Both versions are kept, and the app or the user picks one (STORAGE.md §2). |
| A shared computer | Separate profiles, each with a lock (the OS account or a PIN). |

## 7. Build order

Each step is useful on its own:

1. **Plain words and defaults:** the UI words of §2 and the defaults of §4 in the current Hub and consent screens. *Status (2026-10-06): the words are done in the Hub app ("Your apps", "Store", "Permissions", "Keep", "Allow: <what it does>") and the web consent window; the desktop consent window already used plain words. The defaults of §4 that exist today (low risk allowed with no prompt, medium and high asked) were already in place; the others need the Privacy page and sync.*
2. **Try and Keep:** browse mode in the Hub and the web Hub (scenarios A and B).
3. **Privacy page:** permissions with last use, storage size, Remove, the "reads AND sends" mark, the 90-day expiry (G, H).
4. **Sync through a synced folder**, with end-to-end encryption, the recovery kit and device pairing (C, D, E simple path).
5. **Accounts** (instances, F).
6. **Advanced storage:** S3 and Azure (E advanced).
7. **Publisher domain verification** (J).
8. **Work profiles and policies** (K), then family profiles (L).
9. **Sharing between people** (I).

## 8. Open questions

- UX-Q1: Is "Try" data really lost after the session, or kept for a short time (for example 7 days) so that "Keep" can bring it back?
- UX-Q2: The 90-day expiry: the right time, and does it apply to low-risk permissions?
- UX-Q3: Can the web Hub be a full member of a profile (scenario C in a browser), given that browsers have no keychain (STORAGE.md ST-Q8)?
- UX-Q4: Who runs the block list and the catalog review, and how does a Hub get updates to them without a central server becoming a single point of control?
