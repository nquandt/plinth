//! Each core minor after 1.12 has a migration guide (`docs/migrations/`,
//! PRODUCTION X5), and the index lists it. Core 1.12 and older shipped
//! together in the first public release and have no guide.

use plinth_link::rt_abi::{CORE_MAJOR, CORE_MINOR};
use std::path::PathBuf;

/// The last minor version without a guide.
const LAST_WITHOUT_GUIDE: u32 = 12;

#[test]
fn each_core_minor_has_a_migration_guide() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/migrations");
    let index = std::fs::read_to_string(dir.join("README.md")).expect("docs/migrations/README.md");
    for minor in LAST_WITHOUT_GUIDE + 1..=CORE_MINOR {
        let name = format!("core-{CORE_MAJOR}.{minor}.md");
        assert!(
            dir.join(&name).is_file(),
            "core {CORE_MAJOR}.{minor} has no migration guide: write docs/migrations/{name} (docs/migrations/README.md tells how)"
        );
        assert!(index.contains(&format!("]({name})")), "docs/migrations/README.md does not list {name}");
    }
}
