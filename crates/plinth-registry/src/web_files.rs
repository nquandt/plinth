//! The browser files that `plinth registry serve --web` serves under
//! `/web/` (`docs/web-hub.md`): the web App Hub landing page and the web
//! host (`web/`). They are embedded in the binary, so the server works from
//! any working directory. Tests (`web/test/`) are not embedded.

/// `(file name, bytes)`, for every file under `/web/`.
pub const FILES: &[(&str, &[u8])] = &[
    ("hub.html", include_bytes!("../../../web/hub.html")),
    ("hub-shell.js", include_bytes!("../../../web/hub-shell.js")),
    ("registry-client.js", include_bytes!("../../../web/registry-client.js")),
    ("hub-host.js", include_bytes!("../../../web/hub-host.js")),
    ("hub-storage.js", include_bytes!("../../../web/hub-storage.js")),
    ("hub-integrity.js", include_bytes!("../../../web/hub-integrity.js")),
    ("app-frame.html", include_bytes!("../../../web/app-frame.html")),
    ("app-frame.js", include_bytes!("../../../web/app-frame.js")),
    ("index.html", include_bytes!("../../../web/index.html")),
    ("plinth-web.js", include_bytes!("../../../web/plinth-web.js")),
    ("dom-renderer.js", include_bytes!("../../../web/dom-renderer.js")),
    ("protocol.js", include_bytes!("../../../web/protocol.js")),
    ("zip.js", include_bytes!("../../../web/zip.js")),
    ("ui-api.js", include_bytes!("../../../web/ui-api.js")),
    ("style.css", include_bytes!("../../../web/style.css")),
];

/// The embedded file `name` (for example `hub.html`), if there is one.
pub fn get(name: &str) -> Option<&'static [u8]> {
    FILES.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}
