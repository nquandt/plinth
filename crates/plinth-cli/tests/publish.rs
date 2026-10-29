//! e2e: `plinth publish` (`docs/REGISTRY-HOSTING.md` §2, `docs/PRODUCTION.md`
//! T6) from a scratch copy of `examples/counter` into a temp registry. No
//! network upload runs; the sync test uses a command that writes a file.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_dir(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("plinth-publish-e2e-{name}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let name = entry.file_name();
        if name == "dist" || name == ".plinth" {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        if src.is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

struct Setup {
    root: PathBuf,
    app: PathBuf,
    registry: PathBuf,
    key: PathBuf,
    config: PathBuf,
}

impl Setup {
    /// A scratch copy of the counter (publisher `Acme`), an empty registry
    /// folder and a new `Acme` key.
    fn new(test: &str) -> Self {
        let root = temp_dir(test);
        let app = root.join("counter");
        copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter"), &app);
        let s = Self { app, registry: root.join("registry"), key: root.join("key"), config: root.join("publish.toml"), root };
        s.set_toml("0.1.0", "Acme");
        let out = s.plinth(&["publisher", "init", "--name", "Acme"], &s.app);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        s
    }

    fn set_toml(&self, version: &str, publisher: &str) {
        std::fs::write(
            self.app.join("plinth.toml"),
            format!("id = \"dev.plinth.examples.counter\"\nname = \"Counter\"\nversion = \"{version}\"\npublisher = \"{publisher}\"\n"),
        )
        .unwrap();
    }

    fn plinth(&self, args: &[&str], cwd: &Path) -> Output {
        Command::new(env!("CARGO_BIN_EXE_plinth"))
            .args(args)
            .current_dir(cwd)
            .env("PLINTH_PUBLISHER_DIR", &self.key)
            .env("PLINTH_PUBLISH_CONFIG", &self.config)
            .env_remove("PLINTH_REGISTRY_DIR")
            .output()
            .unwrap()
    }

    fn publish(&self, extra: &[&str]) -> Output {
        let mut args = vec!["publish", self.app.to_str().unwrap(), "--registry", self.registry.to_str().unwrap()];
        args.extend_from_slice(extra);
        self.plinth(&args, &self.root)
    }

    fn versions(&self) -> Vec<String> {
        let id = "dev.plinth.examples.counter";
        let list = std::fs::read_to_string(self.registry.join("apps/index.json")).unwrap();
        assert!(list.contains(id), "the app list has no counter: {list}");
        let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(self.registry.join("apps").join(id).join("index.json")).unwrap()).unwrap();
        doc["versions"].as_array().unwrap().iter().map(|v| v["version"].as_str().unwrap().to_owned()).collect()
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

fn text(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn publish_two_versions_and_repeat() {
    let s = Setup::new("versions");
    let out = s.publish(&[]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(s.registry.join("incoming/counter.plnt").exists());
    assert!(text(&out).contains("added dev.plinth.examples.counter@0.1.0"), "{}", text(&out));
    assert_eq!(s.versions(), ["0.1.0"]);

    // The "built" line names the registry package, not the scratch folder.
    let built = text(&out).lines().find(|l| l.starts_with("built ")).expect("a built line").to_owned();
    let target = s.registry.join("incoming").join("counter.plnt");
    let target = target.to_string_lossy();
    assert!(built.contains(target.as_ref()), "{built}");
    assert_eq!(text(&out).lines().filter(|l| l.starts_with("built ") && l.contains(".plnt")).count(), 1, "{}", text(&out));

    // The same content again: nothing changes, and it is not an error.
    let out = s.publish(&[]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("unchanged dev.plinth.examples.counter@0.1.0"), "{}", text(&out));

    // A second version: both are listed.
    s.set_toml("0.2.0", "Acme");
    let out = s.publish(&["--with-core"]);
    assert!(out.status.success(), "{}", text(&out));
    let mut versions = s.versions();
    versions.sort();
    assert_eq!(versions, ["0.1.0", "0.2.0"]);
    assert!(s.registry.join("cores").exists());
}

#[test]
fn same_version_with_other_content_is_refused_and_changes_nothing() {
    let s = Setup::new("conflict");
    assert!(s.publish(&[]).status.success());
    let before = std::fs::read(s.registry.join("incoming/counter.plnt")).unwrap();
    let index_before = std::fs::read(s.registry.join("apps/dev.plinth.examples.counter/index.json")).unwrap();

    // The same version, other app code.
    let main = ["app/main.tsx", "app/main.ts", "app/app.tsx", "app/index.tsx"].map(|f| s.app.join(f)).into_iter().find(|p| p.exists());
    let main = main.unwrap_or_else(|| first_source(&s.app.join("app")));
    let mut src = std::fs::read_to_string(&main).unwrap();
    src.push_str("\nconst changedForTest = 42;\nexport { changedForTest };\n");
    std::fs::write(&main, src).unwrap();

    let out = s.publish(&[]);
    assert!(!out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("a different package is already registered"), "{}", text(&out));
    // The refusal happens before any write to the registry.
    assert_eq!(std::fs::read(s.registry.join("incoming/counter.plnt")).unwrap(), before);
    assert_eq!(std::fs::read(s.registry.join("apps/dev.plinth.examples.counter/index.json")).unwrap(), index_before);
}

fn first_source(dir: &Path) -> PathBuf {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "ts" || e == "tsx")).collect();
    files.sort();
    files.remove(0)
}

#[test]
fn publisher_must_equal_the_key_name() {
    let s = Setup::new("name");
    s.set_toml("0.1.0", "Someone Else");
    let out = s.publish(&[]);
    assert!(!out.status.success());
    assert!(text(&out).contains("`Someone Else`") && text(&out).contains("`Acme`"), "{}", text(&out));
    assert!(!s.registry.exists(), "a refused publish must not make the registry folder");
}

#[test]
fn check_writes_nothing_to_the_registry() {
    let s = Setup::new("check");
    let out = s.publish(&["--check"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("add dev.plinth.examples.counter@0.1.0"), "{}", text(&out));
    assert!(!s.registry.exists(), "--check made {}", s.registry.display());

    assert!(s.publish(&[]).status.success());
    let out = s.publish(&["--check"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("keep dev.plinth.examples.counter@0.1.0"), "{}", text(&out));
    assert!(!s.publish(&["--check", "--sync"]).status.success());
}

#[test]
fn registry_from_env_and_config_and_sync_command() {
    let s = Setup::new("config");
    let marker = s.root.join("synced.txt");
    let from_config = s.root.join("from-config");
    let from_env = s.root.join("from-env");
    let command = if cfg!(windows) { "echo ok> synced.txt".to_owned() } else { "echo ok > synced.txt".to_owned() };
    std::fs::write(
        &s.config,
        format!(
            "registry_dir = '{}'\nsync_command = '{}'\nregistry_url = \"https://registry.example\"\n",
            from_config.display(),
            command
        ),
    )
    .unwrap();

    // The config gives the registry folder; --sync runs the command there.
    let app = s.app.to_str().unwrap();
    let out = s.plinth(&["publish", app, "--sync"], &s.registry.parent().unwrap().to_path_buf());
    assert!(out.status.success(), "{}", text(&out));
    assert!(from_config.join("plinth-registry.json").exists());
    assert!(text(&out).contains("https://registry.example"), "{}", text(&out));
    assert!(text(&out).contains("sync: "), "{}", text(&out));
    // The command ran with the registry folder in PLINTH_REGISTRY_DIR, in the current folder.
    assert!(s.root.join("synced.txt").exists() && marker.exists());

    // The environment beats the config.
    let out = Command::new(env!("CARGO_BIN_EXE_plinth"))
        .args(["publish", app])
        .env("PLINTH_PUBLISHER_DIR", &s.key)
        .env("PLINTH_PUBLISH_CONFIG", &s.config)
        .env("PLINTH_REGISTRY_DIR", &from_env)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out));
    assert!(from_env.join("plinth-registry.json").exists());
}

#[test]
fn sync_without_a_command_says_so() {
    let s = Setup::new("nosync");
    let out = s.publish(&["--sync"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("sync_command"), "{}", text(&out));
    assert!(!s.registry.exists(), "a refused sync must not change the registry");
}
