//! End-to-end CLI flow for `plinth hub uninstall`, `plinth hub export` and
//! `plinth hub policy quota` (`docs/STORAGE.md` S13): the real `plinth`
//! binary, with `PLINTH_HUB_DIR` and the data directory (`APPDATA` on Windows,
//! `XDG_DATA_HOME` elsewhere) pointed at temp directories. Every test uses a
//! unique app id, so the system keychain never holds a name of a real app.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(name: &str) -> String {
    format!("{name}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

struct Env {
    hub_dir: PathBuf,
    /// The value of `APPDATA` / `XDG_DATA_HOME`.
    base: PathBuf,
    id: String,
}

impl Env {
    fn new(name: &str) -> Env {
        let root = std::env::temp_dir().join(format!("plinth-cli-s13-{}", unique(name)));
        let id = format!("plinth.test.s13.{}", unique(name));
        let env = Env { hub_dir: root.join("hub"), base: root.join("base"), id };
        std::fs::create_dir_all(&env.hub_dir).unwrap();
        std::fs::create_dir_all(&env.base).unwrap();
        env
    }

    /// `<base>/plinth`, the data directory the child process uses.
    fn data_dir(&self) -> PathBuf {
        self.base.join("plinth")
    }

    fn plinth(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_plinth"))
            .env("PLINTH_HUB_DIR", &self.hub_dir)
            .env("APPDATA", &self.base)
            .env("XDG_DATA_HOME", &self.base)
            .env("HOME", &self.base)
            .env_remove("PLINTH_DATA_QUOTA_MIB")
            .args(args)
            .output()
            .unwrap()
    }

    /// Installs a package with the unique id and returns the Hub.
    fn install(&self) -> plinth_hub::Hub {
        let file = self.hub_dir.join("app.plnt");
        std::fs::write(&file, package(&self.id)).unwrap();
        let out = self.plinth(&["hub", "add", file.to_str().unwrap()]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        plinth_hub::Hub::open(&self.hub_dir).unwrap()
    }

    /// Makes data of every kind for the app, with the layout the host uses.
    fn put_data(&self, hub: &plinth_hub::Hub) -> plinth_hub::AppDataLayout {
        let layout = hub.data_layout(&self.id, &self.data_dir()).unwrap();
        std::fs::create_dir_all(&layout.kv).unwrap();
        std::fs::write(layout.kv.join("kv.json"), r#"{"theme":"dark"}"#).unwrap();
        let space = &layout.spaces[0];
        std::fs::create_dir_all(space.files.join("notes")).unwrap();
        std::fs::write(space.files.join("notes/a.md"), "# a").unwrap();
        let db = plinth_runner_wasmtime::sql::Db::open(&space.database, plinth_runner_wasmtime::sql::Limits::default()).unwrap();
        db.exec("CREATE TABLE t (n INTEGER)", "").unwrap();
        db.exec("INSERT INTO t VALUES (42)", "").unwrap();
        drop(db);
        layout
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = plinth_host_desktop::secrets::remove_app_secrets(&self.id);
        if let Some(root) = self.hub_dir.parent() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

fn package(id: &str) -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter");
    let fs = plinth_compiler::driver::DiskFs { root };
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("counter has errors:\n{}", diags.join("\n"))
    });
    let component = artifact.app;
    let cfg = plinth_package::ProjectConfig::parse(&format!("id = \"{id}\"\nname = \"S13\"\nversion = \"0.4.2\"\npublisher = \"Acme\"\n")).unwrap();
    let manifest = cfg.manifest("1.0", "plinth-rt/1.0", None, &component);
    plinth_package::Package::new(manifest, component, Vec::new()).write().unwrap()
}

fn text(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

/// True when the system keychain accepts a write (a CI host can have none).
fn keychain_works(probe_app: &str) -> bool {
    use plinth_runner_wasmtime::secrets::SecretsScope;
    let ok = plinth_host_desktop::secrets::system_secrets().set(&SecretsScope::new(probe_app), "probe", "x").is_ok();
    if !ok {
        eprintln!("skip: no usable system keychain on this host");
    }
    ok
}

#[test]
fn uninstall_with_data_deletes_everything_the_app_owns() {
    let env = Env::new("all");
    let hub = env.install();
    let layout = env.put_data(&hub);
    let secrets = keychain_works(&env.id);
    {
        use plinth_runner_wasmtime::secrets::SecretsScope;
        if secrets {
            plinth_host_desktop::secrets::system_secrets().set(&SecretsScope::new(&env.id), "token", "s3cret").unwrap();
        }
    }
    // Data of another app must stay.
    let other_dir = env.data_dir().join("apps").join(format!("{}.other", env.id));
    std::fs::create_dir_all(&other_dir).unwrap();
    std::fs::write(other_dir.join("kv.json"), "{}").unwrap();

    let out = env.plinth(&["hub", "uninstall", &env.id]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("uninstalled"), "{}", text(&out));
    assert!(!layout.exists(), "kv, files and database are gone");
    assert!(hub.get(&env.id).unwrap().is_none());
    assert!(hub.grants(&env.id).unwrap().is_empty());
    assert!(other_dir.join("kv.json").exists(), "another app keeps its data");
    if secrets {
        use plinth_runner_wasmtime::secrets::SecretsScope;
        let mut backend = plinth_host_desktop::secrets::system_secrets();
        assert_eq!(backend.names(&SecretsScope::new(&env.id)), Ok(vec![]), "the secrets are gone");
        assert_eq!(backend.get(&SecretsScope::new(&env.id), "token"), Ok(None));
    }
}

#[test]
fn uninstall_keep_data_leaves_the_data_and_the_secrets() {
    let env = Env::new("keep");
    let hub = env.install();
    let layout = env.put_data(&hub);
    let out = env.plinth(&["hub", "uninstall", &env.id, "--keep-data"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("stay"), "{}", text(&out));
    assert!(layout.exists());
    assert!(hub.get(&env.id).unwrap().is_none());
}

#[test]
fn uninstall_refuses_while_the_app_runs() {
    let env = Env::new("busy");
    let hub = env.install();
    let layout = env.put_data(&hub);
    let lock = plinth_hub::RunLock::running(&env.data_dir(), &env.id).unwrap();
    let out = env.plinth(&["hub", "uninstall", &env.id]);
    assert!(!out.status.success());
    assert!(text(&out).contains("is running"), "{}", text(&out));
    assert!(layout.exists() && hub.get(&env.id).unwrap().is_some(), "nothing changed");
    drop(lock);
    let out = env.plinth(&["hub", "uninstall", &env.id]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(!layout.exists());
}

#[test]
fn uninstall_and_export_refuse_ids_that_look_like_paths() {
    let env = Env::new("path");
    let victim = env.base.join("victim");
    std::fs::create_dir_all(&victim).unwrap();
    std::fs::write(victim.join("precious.txt"), "x").unwrap();
    for bad in ["..", "../victim", "..\\victim", "a/b"] {
        let out = env.plinth(&["hub", "uninstall", bad]);
        assert!(!out.status.success(), "{bad}");
        assert!(text(&out).contains("not a valid app id"), "{bad}: {}", text(&out));
        let out = env.plinth(&["hub", "export", bad, env.base.join("x.zip").to_str().unwrap()]);
        assert!(!out.status.success(), "{bad}");
    }
    assert!(victim.join("precious.txt").exists());
}

#[test]
fn export_writes_the_data_and_says_that_secrets_are_not_in_it() {
    let env = Env::new("export");
    let hub = env.install();
    env.put_data(&hub);
    let out_zip = env.base.join("out").join("export.zip");
    let out = env.plinth(&["hub", "export", &env.id, out_zip.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("secrets of the app are not in the export"), "{}", text(&out));

    let mut archive = zip::ZipArchive::new(std::fs::File::open(&out_zip).unwrap()).unwrap();
    let names: Vec<String> = archive.file_names().map(|n| n.unwrap().into_owned()).collect();
    assert!(names.contains(&"manifest.json".to_owned()), "{names:?}");
    assert!(names.contains(&"kv.json".to_owned()), "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("files/") && n.ends_with("/notes/a.md")), "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("sql/") && n.ends_with("/store.sqlite")), "{names:?}");
    let mut manifest = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("manifest.json").unwrap(), &mut manifest).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(manifest["app"], env.id.as_str());
    assert_eq!(manifest["version"], "0.4.2");
    assert_eq!(manifest["secrets"], "not included");
    // The export is a copy: the app and its data are still there.
    assert!(hub.get(&env.id).unwrap().is_some());
    assert!(hub.data_layout(&env.id, &env.data_dir()).unwrap().exists());
}

#[test]
fn the_quota_command_sets_and_clears_the_policy() {
    let env = Env::new("quota");
    let show = text(&env.plinth(&["hub", "policy", "show"]));
    assert!(show.contains("100.0 MiB"), "{show}");
    let out = env.plinth(&["hub", "policy", "quota", "7"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&env.plinth(&["hub", "policy", "show"])).contains("7.0 MiB"));
    assert!(!env.plinth(&["hub", "policy", "quota", "0"]).status.success());
    assert!(!env.plinth(&["hub", "policy", "quota", "lots"]).status.success());
    assert!(env.plinth(&["hub", "policy", "quota", "default"]).status.success());
    assert!(text(&env.plinth(&["hub", "policy", "show"])).contains("100.0 MiB"));
    // The policy file is the host config that a later app start reads.
    assert_eq!(plinth_hub::data_quota_in(&env.hub_dir), 100 << 20);
}
