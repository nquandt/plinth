# iOS web shell (untested bridge)

This folder is a small iOS app that opens the web App Hub (`https://plinth.femur.dev/`) in a `WKWebView`. It exists so a Plinth app can run on an iPhone before the native gpui host exists. It is written on Windows and **has never been built**: expect small fixes. Read `docs/MAC-HANDOFF.md` (track A).

```sh
brew install xcodegen
xcodegen generate
open PlinthHub.xcodeproj    # set the team, pick the iPhone, press Run
```

It is a bridge, not the product: the owner decided that mobile hosts render with gpui (`HANDOFF.md` §0.1). One `.plnt` runs in both.
