//! Generates `ids.rs` from `wit/plinth/ui-api.toml`.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let table_path = manifest_dir.join("../../wit/plinth/ui-api.toml");
    println!("cargo:rerun-if-changed={}", table_path.display());

    let source = std::fs::read_to_string(&table_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", table_path.display()));
    let table: toml::Table = source.parse().expect("ui-api.toml is not valid TOML");

    let mut out = String::new();
    let version = table["version"].as_str().expect("version must be a string");
    writeln!(out, "/// The UI API version of the id table.").unwrap();
    writeln!(out, "pub const UI_API_VERSION: &str = {version:?};\n").unwrap();

    // Control kinds become a Rust enum, because the host matches on them.
    let controls = sorted(table["controls"].as_table().expect("[controls]"));
    check_unique("controls", &controls);
    writeln!(out, "/// A control kind (the `kind` of a `create` op).").unwrap();
    writeln!(out, "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\n#[repr(u16)]").unwrap();
    writeln!(out, "pub enum ControlKind {{").unwrap();
    for (name, id) in &controls {
        writeln!(out, "    {} = {id},", pascal(name)).unwrap();
    }
    writeln!(out, "}}\n").unwrap();
    writeln!(out, "impl ControlKind {{").unwrap();
    writeln!(out, "    pub fn from_u16(v: u16) -> Option<Self> {{\n        match v {{").unwrap();
    for (name, id) in &controls {
        writeln!(out, "            {id} => Some(Self::{}),", pascal(name)).unwrap();
    }
    writeln!(out, "            _ => None,\n        }}\n    }}\n").unwrap();
    writeln!(out, "    pub fn name(self) -> &'static str {{\n        match self {{").unwrap();
    for (name, _) in &controls {
        writeln!(out, "            Self::{} => {name:?},", pascal(name)).unwrap();
    }
    writeln!(out, "        }}\n    }}\n}}\n").unwrap();

    // Props, events and enums become `u16` constants in a module each.
    write_consts(&mut out, "prop", "Prop ids (`set-prop`).", &table["props"]);
    write_consts(&mut out, "event", "Event ids (`listen` and `ui` events).", &table["events"]);
    for (name, values) in table["enums"].as_table().expect("[enums]") {
        write_consts(&mut out, &snake(name), &format!("Values of the `{name}` enum."), values);
    }

    let out_path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ids.rs");
    std::fs::write(out_path, out).unwrap();
}

fn write_consts(out: &mut String, module: &str, doc: &str, value: &toml::Value) {
    let entries = sorted(value.as_table().expect("table"));
    check_unique(module, &entries);
    writeln!(out, "/// {doc}\npub mod {module} {{").unwrap();
    for (name, id) in &entries {
        writeln!(out, "    pub const {}: u16 = {id};", snake(name).to_uppercase()).unwrap();
    }
    writeln!(out, "\n    pub fn name(id: u16) -> Option<&'static str> {{\n        match id {{").unwrap();
    for (name, id) in &entries {
        writeln!(out, "            {id} => Some({name:?}),").unwrap();
    }
    writeln!(out, "            _ => None,\n        }}\n    }}\n}}\n").unwrap();
}

fn sorted(table: &toml::Table) -> Vec<(String, u16)> {
    let mut entries: Vec<(String, u16)> = table
        .iter()
        .map(|(k, v)| {
            let id = v.as_integer().unwrap_or_else(|| panic!("{k}: id must be an integer"));
            (k.clone(), u16::try_from(id).unwrap_or_else(|_| panic!("{k}: id out of range")))
        })
        .collect();
    entries.sort_by_key(|(_, id)| *id);
    entries
}

fn check_unique(section: &str, entries: &[(String, u16)]) {
    for pair in entries.windows(2) {
        assert!(pair[0].1 != pair[1].1, "{section}: `{}` and `{}` share id {}", pair[0].0, pair[1].0, pair[0].1);
    }
}

fn snake(name: &str) -> String {
    name.replace('-', "_")
}

fn pascal(name: &str) -> String {
    name.split('-')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map(|c| c.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
        })
        .collect()
}
