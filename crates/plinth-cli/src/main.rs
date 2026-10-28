//! The `plinth` command-line tool (SPEC.md §13).

use anyhow::{Context as _, Result, bail};
use plinth_compiler::driver::{DiskFs, Frontend};
use plinth_package::{Package, ProjectConfig};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

mod dev_tools;
mod feed_cmd;
mod optimize;
mod publish_cmd;
mod registry_cmd;
mod test_cmd;
mod web_export;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
plinth - build cross-platform apps from Plinth TS

usage:
  plinth new <dir>                 make a new app (hello world)
  plinth dev [dir]                 run the app; reload it when a file changes
  plinth check [dir] [--json] [--watch]
                                   type-check, with no build
  plinth build [dir] [--out <file>] [--sign] [--release]
                                   make dist/<name>.plnt (--sign adds signature.json;
                                   --release makes the app code smaller with wasm-opt)
  plinth test [dir] [--filter <text>]
                                   run the tests in app/**/*.test.ts(x) (docs/TESTING.md)
  plinth build [dir] --target web [--single-file] [--out <path>] [--release]
                                   make dist/web/ (index.html, plinth.js, the core,
                                   the package) for any static web host, or
                                   dist/<name>.html with everything inline
  plinth sign <file.plnt>          add signature.json to an existing package
  plinth publisher init [--name <publisher>]
                                   create a publisher key pair (docs/HUB.md §6.1)
  plinth publisher show            print the publisher name and public key id
  plinth run <app.plnt | app.wasm> run a package
  plinth native <app.plnt | dir> [-o <file>] [--release]
                                   make one executable: this host and the app
  plinth validate <app.plnt | app.wasm>
                                   check a package and its Wasm imports
  plinth core list                 show the installed runtime cores
  plinth core install <core.wasm>  install a runtime core
  plinth core export <file>        write the built-in core to a file
  plinth hub add <app.plnt>        add a package to the Hub library
  plinth hub list                  list library apps
  plinth hub run <app id>          run a library app (consent screen first)
  plinth hub ui [<app id>]         run the Hub UI app (default dev.plinth.hub)
  plinth hub open <plinth://app/<id>>  open an app link (installs from sources if needed)
  plinth hub register-scheme [--exe <path>] | unregister-scheme
                                   register plinth:// links for this user (Windows)
  plinth hub shortcut <app id> [--start-menu] [--dir <folder>] [--url]
                                   write a shortcut (.lnk, app icon) that opens the app
  plinth hub remove <app id>       remove an app from the library (its data stays)
  plinth hub uninstall <app id> [--keep-data]
                                   remove an app and, unless --keep-data, all its data
                                   (kv, sql, files, grants, secrets); refuses while it runs
  plinth hub export <app id> <out.zip>
                                   write the data of an app to a zip (no secrets)
  plinth hub grants <app id> [allow|refuse <capability>]
                                   show or set a grant
  plinth hub block|unblock <app id>
                                   block or unblock an app
  plinth hub block-publisher|unblock-publisher <key id>
                                   block or unblock a publisher (all its apps)
  plinth hub groups [create <name> | add <app id> <name>]
                                   list, or manage, library groups
  plinth hub source add <name> <base> | list | remove <name>
  plinth hub source set-token <name> [--client-id <id>] [--client-secret <secret>] | clear-token <name>
  plinth hub source trust-hub <name> [--yes]
                                   read hub.json of a source; with --yes save its Hub keys
                                   as trusted (without --yes it only shows them)
                                   manage registry sources (docs/REGISTRY.md)
  plinth hub search <text>         search configured registry sources
  plinth hub install <id>[@version] [--source <name>]
                                   download and add an app to the library
  plinth hub update [<id>]         install newer versions from the app's source
  plinth hub pin <app id> <version> | --latest
                                   run one installed version, or the newest again
  plinth hub policy deny|allow <capability> | show
                                   a hub-wide switch for a capability
  plinth hub policy trust-hub-key|untrust-hub-key <key id>
                                   trust (or stop to trust) a key id as a Hub key: only a
                                   package signed by it may use hub.manage (docs/HUB.md §4.1)
  plinth hub policy quota <MiB> | default
                                   the data quota of one app for each store (default 100 MiB)
  plinth registry build <folder> [--with-core] [--hub-trusted-key <key id>]
                                   generate a static registry from .plnt files
  plinth registry serve <folder> [--port N] [--web]
                                   serve a static registry on 127.0.0.1;
                                   --web also serves the web App Hub at /
  plinth publish [dir] [--registry <folder>] [--with-core] [--sync] [--check]
                                   build with --sign --release into a registry folder and
                                   update its index (docs/REGISTRY-HOSTING.md §2); --check
                                   builds and validates and writes nothing to the registry
  plinth feed add <feed-dir> <app.plnt> [--channel <name>] [--with-core]
                                   write or update a signed update feed (docs/UPDATES.md §8)

The project directory defaults to the current directory.";

/// The typings that `plinth` writes to `.plinth/types` (SPEC.md §4.7).
const TYPINGS: &[(&str, &str)] = &[
    ("lib.d.ts", include_str!("../../../std/lib.d.ts")),
    ("ui.d.ts", include_str!("../../../std/ui.d.ts")),
    ("core.d.ts", include_str!("../../../std/core.d.ts")),
    ("time.d.ts", include_str!("../../../std/time.d.ts")),
    ("store.d.ts", include_str!("../../../std/store.d.ts")),
    ("clipboard.d.ts", include_str!("../../../std/clipboard.d.ts")),
    ("dialog.d.ts", include_str!("../../../std/dialog.d.ts")),
    ("net.d.ts", include_str!("../../../std/net.d.ts")),
    ("files.d.ts", include_str!("../../../std/files.d.ts")),
    ("notify.d.ts", include_str!("../../../std/notify.d.ts")),
    ("secrets.d.ts", include_str!("../../../std/secrets.d.ts")),
    ("locale.d.ts", include_str!("../../../std/locale.d.ts")),
    ("test.d.ts", include_str!("../../../std/test.d.ts")),
];

/// The `std/*.d.ts` files that `TYPINGS` leaves out, each with the reason.
/// A test checks that every file is in one of the two lists.
#[cfg(test)]
const TYPINGS_LEFT_OUT: &[(&str, &str)] = &[(
    "hub.d.ts",
    "plinth:hub needs `hub.manage`, which the host gives only to packages signed with a trusted Hub key; a normal app cannot use it",
)];

fn main() -> ExitCode {
    // A single-file export (SPEC.md §10.3) runs its payload and nothing else.
    match std::env::current_exe().map_err(anyhow::Error::from).and_then(|exe| plinth_package::read_payload(&exe)) {
        Ok(Some(plnt)) => return run_payload(plnt),
        Ok(None) => {}
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::FAILURE;
        }
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<ExitCode> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (flags, positional): (Vec<&str>, Vec<&str>) = args.iter().partition(|a| a.starts_with("--"));
    let flag = |f: &str| flags.contains(&f);
    let dir = |i: usize| PathBuf::from(positional.get(i).copied().unwrap_or("."));
    match positional.first().copied() {
        Some("new") => {
            let target = positional.get(1).context("usage: plinth new <dir>")?;
            new_project(Path::new(target))?;
            Ok(ExitCode::SUCCESS)
        }
        Some("check") => {
            if flag("--watch") {
                watch_check(&dir(1))
            } else {
                Ok(if check(&dir(1), flag("--json"))? { ExitCode::SUCCESS } else { ExitCode::FAILURE })
            }
        }
        Some("build") => {
            let value = |f: &str| args.iter().position(|a| *a == f).and_then(|i| args.get(i + 1)).copied();
            let (out, target) = (value("--out"), value("--target"));
            let project = positional[1..].iter().find(|p| Some(**p) != out && Some(**p) != target).map(PathBuf::from);
            let project = project.unwrap_or_else(|| PathBuf::from("."));
            let out = out.map(PathBuf::from);
            let ok = match target {
                None => build(&project, out, flag("--sign"), flag("--release"))?,
                Some("web") => build_web(&project, out, flag("--single-file"), flag("--sign"), flag("--release"))?,
                Some(t) => bail!("unknown target `{t}`; use `--target web`, or `plinth native` for an executable"),
            };
            Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Some("publish") => {
            let registry = args.iter().position(|a| *a == "--registry").and_then(|i| args.get(i + 1)).copied();
            if flag("--registry") && registry.is_none_or(|r| r.starts_with("--")) {
                bail!("--registry needs a folder");
            }
            let app_dir = positional[1..].iter().find(|p| Some(**p) != registry).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            let options = publish_cmd::Options {
                app_dir,
                registry: registry.map(PathBuf::from),
                with_core: flag("--with-core"),
                sync: flag("--sync"),
                check: flag("--check"),
            };
            Ok(if publish_cmd::run(&options)? { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Some("test") => {
            let filter = args.iter().position(|a| *a == "--filter").and_then(|i| args.get(i + 1)).copied();
            let project = positional[1..].iter().find(|p| Some(**p) != filter).map(PathBuf::from);
            let project = project.unwrap_or_else(|| PathBuf::from("."));
            Ok(if test_cmd::run(&project, filter)? { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Some("sign") => {
            let file = positional.get(1).context("usage: plinth sign <file.plnt>")?;
            sign_file(Path::new(file))?;
            Ok(ExitCode::SUCCESS)
        }
        Some("publisher") => publisher_command(&positional[1..], &args[1..]),
        Some("dev") => dev(&dir(1)),
        Some("run") => {
            let file = positional.get(1).context("usage: plinth run <app.plnt | app.wasm>")?;
            plinth_host_desktop::init_logging();
            plinth_host_desktop::run(plinth_host_desktop::HostApp::load(Path::new(file))?, None)?;
            Ok(ExitCode::SUCCESS)
        }
        Some("native") => {
            let input = positional.get(1).context("usage: plinth native <app.plnt | dir> [-o <file>]")?;
            let out = args.iter().position(|a| *a == "-o").and_then(|i| args.get(i + 1)).map(PathBuf::from);
            if let Some(t) = args.iter().position(|a| *a == "-t").and_then(|i| args.get(i + 1))
                && *t != std::env::consts::OS
            {
                bail!("this plinth can make `{}` executables only; run `plinth native` on {t}", std::env::consts::OS);
            }
            Ok(if native(Path::new(input), out, flag("--release"))? { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Some("core") => {
            use plinth_compiler::cores;
            match positional.get(1).copied() {
                Some("list") | None => {
                    println!("cores directory: {}", cores::cores_dir().display());
                    for c in cores::installed() {
                        let at = c.path.map(|p| p.display().to_string()).unwrap_or_else(|| "built in".into());
                        println!(
                            "  {}  {} KiB  digest {}…  {at}",
                            cores::format_version(c.version),
                            c.bytes.len().div_ceil(1024),
                            &c.digest[..16]
                        );
                    }
                }
                Some("install") => {
                    let file = positional.get(2).context("usage: plinth core install <core.wasm>")?;
                    let path = cores::install(&std::fs::read(file).with_context(|| format!("read {file}"))?)?;
                    println!("installed {}", path.display());
                }
                Some("export") => {
                    let file = positional.get(2).context("usage: plinth core export <file>")?;
                    std::fs::write(file, plinth_compiler::link::runtime()).with_context(|| format!("write {file}"))?;
                    let version = cores::core_version(plinth_compiler::link::runtime()).unwrap_or_default();
                    println!("wrote core {} to {file}", cores::format_version(version));
                }
                Some(other) => bail!("unknown `plinth core {other}`; use list, install or export"),
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("hub") => hub_command(&positional[1..], &args[1..]),
        Some("registry") => registry_command(&positional[1..], &args[1..]),
        Some("feed") => feed_command(&positional[1..], &args[1..]),
        Some("validate") => {
            let file = positional.get(1).context("usage: plinth validate <file>")?;
            dev_tools::validate_file(Path::new(file))?;
            println!("ok: {file}");
            Ok(ExitCode::SUCCESS)
        }
        // Tools for working on Plinth itself.
        Some("example") => {
            let name = positional.get(1).context("usage: plinth example <rust-guest-crate>")?;
            let out = dev_tools::build_rust_example(name)?;
            println!("{}", out.display());
            Ok(ExitCode::SUCCESS)
        }
        Some("componentize") => {
            let (input, output) = (positional.get(1).context("input")?, args.iter().position(|a| *a == "-o").and_then(|i| args.get(i + 1)).context("-o <out>")?);
            let component = dev_tools::componentize(&std::fs::read(input)?)?;
            dev_tools::validate_component(&component)?;
            std::fs::write(output, component)?;
            Ok(ExitCode::SUCCESS)
        }
        _ if flag("--version") => {
            println!("plinth {VERSION}");
            Ok(ExitCode::SUCCESS)
        }
        _ => {
            println!("{USAGE}");
            Ok(if flag("--help") || positional.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(2) })
        }
    }
}

// -- Projects ------------------------------------------------------------------

fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() { "app".into() } else { s }
}

fn title_case(slug: &str) -> String {
    slug.split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn new_project(dir: &Path) -> Result<()> {
    if dir.exists() && std::fs::read_dir(dir)?.next().is_some() {
        bail!("{} is not empty", dir.display());
    }
    let name_part = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "app".into());
    let slug = slug(&name_part);
    let name = title_case(&slug);
    let id_part = slug.replace('-', "");
    std::fs::create_dir_all(dir.join("app"))?;
    let write = |rel: &str, text: &str| -> Result<()> {
        let p = dir.join(rel);
        std::fs::write(&p, text.replace("{{name}}", &name).replace("{{slug}}", &slug)).with_context(|| format!("write {}", p.display()))
    };
    write("app/main.tsx", include_str!("../templates/hello/main.tsx"))?;
    write("app/model.ts", include_str!("../templates/hello/model.ts"))?;
    write("app/tasks.tsx", include_str!("../templates/hello/tasks.tsx"))?;
    write("app/detail.tsx", include_str!("../templates/hello/detail.tsx"))?;
    write("app/tasks.test.tsx", include_str!("../templates/hello/tasks.test.tsx"))?;
    write("tsconfig.json", include_str!("../templates/tsconfig.json"))?;
    write(".gitignore", include_str!("../templates/gitignore"))?;
    write("README.md", include_str!("../templates/README.md"))?;
    write(
        "plinth.toml",
        &format!(
            "# The app metadata (SPEC.md §10.2).\nid = \"com.example.{id_part}\"\nname = \"{name}\"\nversion = \"0.1.0\"\npublisher = \"example\"\n\n[[capabilities]]\nname = \"store.kv\"\nrationale = \"Save your tasks on this device.\"\n"
        ),
    )?;
    write(
        "package.json",
        &format!(
            "{{\n  \"name\": \"{slug}\",\n  \"version\": \"0.1.0\",\n  \"private\": true,\n  \"scripts\": {{\n    \"dev\": \"plinth dev\",\n    \"check\": \"plinth check\",\n    \"test\": \"plinth test\",\n    \"build\": \"plinth build\",\n    \"build:web\": \"plinth build --target web\"\n  }},\n  \"devDependencies\": {{\n    \"@plinth/cli\": \"^{VERSION}\"\n  }}\n}}\n"
        ),
    )?;
    write_typings(dir)?;
    let shown = dir.display();
    println!("Made a Plinth app in {shown}.\n\nNext:\n  cd {shown}\n  npm install      (or use the plinth binary directly)\n  npm run dev");
    Ok(())
}

/// Writes the `plinth:*` typings for the editor, if they changed.
fn write_typings(project: &Path) -> Result<()> {
    let dir = project.join(".plinth/types");
    std::fs::create_dir_all(&dir)?;
    for (name, text) in TYPINGS {
        let p = dir.join(name);
        if std::fs::read_to_string(&p).ok().as_deref() != Some(*text) {
            std::fs::write(&p, text).with_context(|| format!("write {}", p.display()))?;
        }
    }
    Ok(())
}

/// Reads `<dir>/assets/*` into a map keyed by file name (SPEC.md §10.1).
fn read_assets(dir: &Path) -> std::collections::HashMap<String, Vec<u8>> {
    let mut out = std::collections::HashMap::new();
    let Ok(rd) = std::fs::read_dir(dir.join("assets")) else { return out };
    for e in rd.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Some(name) = e.file_name().to_str().map(str::to_owned) else { continue };
        if let Ok(bytes) = std::fs::read(e.path()) {
            out.insert(name, bytes);
        }
    }
    out
}

fn ensure_project(dir: &Path) -> Result<ProjectConfig> {
    let toml = dir.join("plinth.toml");
    let text = std::fs::read_to_string(&toml)
        .with_context(|| format!("{} has no plinth.toml; run `plinth new <dir>` to make a project", dir.display()))?;
    let config = ProjectConfig::parse(&text)?;
    // Repository examples use the std typings directly.
    if !dir.join("../../std/ui.d.ts").exists() {
        write_typings(dir)?;
    }
    Ok(config)
}

fn print_diags(front: &Frontend) -> (usize, usize) {
    let mut errors = 0;
    let mut warnings = 0;
    for d in &front.diags {
        eprintln!("{}\n", front.sources.render(d));
        match d.severity {
            plinth_compiler::diag::Severity::Error => errors += 1,
            plinth_compiler::diag::Severity::Warning => warnings += 1,
        }
    }
    (errors, warnings)
}

fn summary(errors: usize, warnings: usize) -> String {
    let plural = |n: usize, w: &str| if n == 1 { format!("1 {w}") } else { format!("{n} {w}s") };
    format!("{}, {}", plural(errors, "error"), plural(warnings, "warning"))
}

// -- check ---------------------------------------------------------------------

fn check(dir: &Path, json: bool) -> Result<bool> {
    let config = ensure_project(dir)?;
    let caps: Vec<String> = config.capabilities.iter().map(|c| c.name.clone()).collect();
    let front = plinth_compiler::driver::frontend_with_capabilities(&DiskFs { root: dir.to_path_buf() }, &caps);
    if json {
        println!("{}", front.sources.to_json(&front.diags));
        return Ok(!front.has_errors());
    }
    let (e, w) = print_diags(&front);
    if e == 0 {
        println!("ok ({})", summary(e, w));
    } else {
        eprintln!("{}", summary(e, w));
    }
    Ok(e == 0)
}

fn watch_check(dir: &Path) -> Result<ExitCode> {
    let mut last = None;
    loop {
        let stamp = sources_stamp(dir);
        if Some(stamp) != last {
            last = Some(stamp);
            println!("\n[plinth] checking...");
            if let Err(e) = check(dir, false) {
                eprintln!("error: {e:#}");
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// A fingerprint of the project sources: paths, sizes and times.
fn sources_stamp(dir: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut stack = vec![dir.join("app")];
    let mut entries = Vec::new();
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(m) = e.metadata() {
                entries.push((p, m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
            }
        }
    }
    if let Ok(m) = std::fs::metadata(dir.join("plinth.toml")) {
        entries.push((dir.join("plinth.toml"), m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
    }
    entries.sort();
    entries.hash(&mut h);
    h.finish()
}

// -- build ---------------------------------------------------------------------

struct Built {
    config: ProjectConfig,
    /// The app module for the package: only the app code.
    app: Vec<u8>,
    runtime: String,
    /// The app linked into the runtime, for the dev host.
    component: Vec<u8>,
    accent: Option<String>,
}

fn compile(dir: &Path) -> Result<Option<Built>> {
    compile_ex(dir, false)
}

/// `dev` selects hot reload support (SPEC.md §13): the artifact registers
/// its module-level signals and answers snapshot requests. `plinth dev`
/// passes `true`; every other command passes `false` so release
/// artifacts never contain that code.
fn compile_ex(dir: &Path, dev: bool) -> Result<Option<Built>> {
    let config = ensure_project(dir)?;
    let caps: Vec<String> = config.capabilities.iter().map(|c| c.name.clone()).collect();
    let (front, artifact) = plinth_compiler::compile_ex(&DiskFs { root: dir.to_path_buf() }, &caps, dev)?;
    let (e, w) = print_diags(&front);
    match artifact {
        Some(a) => {
            if w > 0 {
                eprintln!("{}", summary(e, w));
            }
            Ok(Some(Built { config, app: a.app, runtime: a.runtime, component: a.component, accent: a.accent }))
        }
        None => {
            eprintln!("{}", summary(e, w));
            Ok(None)
        }
    }
}

fn build(dir: &Path, out: Option<PathBuf>, sign: bool, release: bool) -> Result<bool> {
    Ok(build_package(dir, out, sign, release)?.is_some())
}

/// Builds the package and returns its path, or `None` when the app has
/// errors. `sign`: adds `signature.json` with the loaded publisher
/// identity (`docs/HUB.md` §6.1); the manifest's `publisher` must equal
/// the identity's name. `release`: optimizes the app module with
/// `wasm-opt` (`optimize.rs`, SPEC.md §5.1 step 10).
fn build_package(dir: &Path, out: Option<PathBuf>, sign: bool, release: bool) -> Result<Option<PathBuf>> {
    let started = std::time::Instant::now();
    let Some(mut b) = compile(dir)? else { return Ok(None) };
    let mut note = String::new();
    if release {
        match optimize::optimize_app(&b.app) {
            Ok(app) => {
                note = format!(", {} B before wasm-opt", b.app.len());
                b.app = app;
            }
            // A failed optimization never stops a build: the package keeps
            // the module that the compiler made.
            Err(e) => eprintln!("warning: the app module is not optimized: {e:#}"),
        }
    }
    // For `core = "pinned"`, the manifest records the digest of the core
    // build this compile used, so a host can find the exact match later
    // (SPEC.md §10.5).
    let core_digest = plinth_compiler::cores::digest_hex(plinth_compiler::link::runtime());
    let mut manifest = b.config.manifest_with_core_digest(plinth_protocol::UI_API_VERSION, &b.runtime, b.accent, &b.app, Some(&core_digest));
    // For `core = "patch"`, the manifest records the version of the core
    // this compile used: the host takes only patches of its minor version
    // (SPEC.md §10.5).
    if b.config.core == plinth_package::CorePolicy::Patch {
        manifest.core_built = plinth_link::cores::core_version(plinth_compiler::link::runtime()).map(plinth_link::cores::format_version);
    }
    let assets: Vec<(String, Vec<u8>)> = read_assets(dir).into_iter().map(|(name, bytes)| (format!("assets/{name}"), bytes)).collect();
    let mut pkg = Package::new(manifest, b.app, assets);
    if sign {
        let identity = plinth_package::publisher::load()?;
        pkg.signature = Some(plinth_package::signature::sign(&pkg, &identity)?);
    }
    let bytes = pkg.write()?;
    let out = out.unwrap_or_else(|| {
        let slug = b.config.id.rsplit('.').next().unwrap_or("app").to_owned();
        dir.join("dist").join(format!("{slug}.{}", plinth_package::EXTENSION))
    });
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &bytes).with_context(|| format!("write {}", out.display()))?;
    println!(
        "built {} in {} ms (app {} B{note}, package {} KiB)",
        out.display(),
        started.elapsed().as_millis(),
        pkg.component.len(),
        bytes.len().div_ceil(1024)
    );
    Ok(Some(out))
}

// -- web export (SPEC.md §10.3) -------------------------------------------------

/// Builds the package, then the web export: the folder `dist/web/` or, with
/// `single_file`, `dist/<name>.html`. `out` replaces the default path.
fn build_web(dir: &Path, out: Option<PathBuf>, single_file: bool, sign: bool, release: bool) -> Result<bool> {
    let Some(plnt_path) = build_package(dir, None, sign, release)? else { return Ok(false) };
    let plnt = std::fs::read(&plnt_path).with_context(|| format!("read {}", plnt_path.display()))?;
    let app = plinth_host_desktop::HostApp::from_bytes(plnt.clone(), &plnt_path)?;
    let core = plinth_compiler::link::runtime();
    let core_version = plinth_compiler::cores::core_version(core).context("the built-in core has no version")?;
    let version = plinth_compiler::cores::format_version(core_version);
    let file = plnt_path.file_name().and_then(|n| n.to_str()).context("package file name")?.to_owned();
    let sql = app.capabilities.iter().any(|c| c == web_export::SQL_CAPABILITY);
    let web = web_export::WebApp { name: &app.title, plnt: (&plnt, &file), core: (core, &version), sql };
    let dist = plnt_path.parent().unwrap_or(Path::new("."));
    if single_file {
        if sql {
            eprintln!(
                "warning: {} declares store.sql, but a single-file web export has no SQL engine: each sql call answers \"denied:unsupported\". Use the folder form (no --single-file) for store.sql.",
                app.title
            );
        }
        let out = out.unwrap_or_else(|| plnt_path.with_extension("html"));
        if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let html = web_export::single_file_html(&web);
        std::fs::write(&out, &html).with_context(|| format!("write {}", out.display()))?;
        println!("made {} ({}, core {version}, {} KiB): open it in a browser, or put it on any web host", out.display(), app.title, html.len().div_ceil(1024));
    } else {
        let out = out.unwrap_or_else(|| dist.join("web"));
        web_export::write_folder(&out, &web)?;
        let with_sql = if sql { ", with the SQLite engine for store.sql" } else { "" };
        println!("made {} ({}, core {version}{with_sql}): put the folder on any static web host", out.display(), app.title);
    }
    Ok(true)
}

// -- native (single-file export, SPEC.md §10.3) ---------------------------------

/// Makes one executable from this host and a package. `input` is a `.plnt`
/// or a project directory, which is built first.
fn native(input: &Path, out: Option<PathBuf>, release: bool) -> Result<bool> {
    let plnt_path = if input.is_dir() {
        match build_package(input, None, false, release)? {
            Some(p) => p,
            None => return Ok(false),
        }
    } else {
        input.to_path_buf()
    };
    let plnt = std::fs::read(&plnt_path).with_context(|| format!("read {}", plnt_path.display()))?;
    // Check the package before it goes into an executable.
    let app = plinth_host_desktop::HostApp::from_bytes(plnt.clone(), &plnt_path)?;
    // The stub is `plinth-host` when it is next to this program (it has no
    // compiler); otherwise this program, which can also run a payload.
    let me = std::env::current_exe()?;
    let runner = me.with_file_name(format!("plinth-host{}", std::env::consts::EXE_SUFFIX));
    let host_exe = if runner.is_file() { runner } else { me };
    if plinth_package::read_payload(&host_exe)?.is_some() {
        bail!("{} is a single-file export; use the plinth CLI", host_exe.display());
    }
    let host = std::fs::read(&host_exe).with_context(|| format!("read {}", host_exe.display()))?;
    let out = out.unwrap_or_else(|| {
        let stem = plnt_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "app".into());
        plnt_path.with_file_name(format!("{stem}{}", std::env::consts::EXE_SUFFIX))
    });
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, plinth_package::append_payload(&host, &plnt)).with_context(|| format!("write {}", out.display()))?;
    println!("made {} ({}, {} MiB)", out.display(), app.title, std::fs::metadata(&out)?.len() >> 20);
    Ok(true)
}

/// Runs the payload of a single-file export, with the updater
/// (`docs/UPDATES.md` §5, §6, §8) if the embedded package has `[updates]`
/// (`plinth_host_desktop::run_payload`).
fn run_payload(plnt: Vec<u8>) -> ExitCode {
    hide_own_console();
    plinth_host_desktop::init_logging();
    let name = std::env::current_exe().unwrap_or_default();
    match plinth_host_desktop::run_payload(plnt, &name) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// On Windows, a console program that starts from Explorer gets its own
/// console window. An app does not need it, so the export closes it. A
/// console that a terminal owns stays.
fn hide_own_console() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
            fn FreeConsole() -> i32;
        }
        let mut ids = [0u32; 2];
        // SAFETY: the buffer holds `ids.len()` process ids.
        if unsafe { GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) } == 1 {
            // SAFETY: no arguments; a process can always leave its console.
            unsafe { FreeConsole() };
        }
    }
}

// -- dev -----------------------------------------------------------------------

fn dev(dir: &Path) -> Result<ExitCode> {
    plinth_host_desktop::init_logging();
    println!("[plinth] dev: {}", dir.display());
    // Wait for the first good build, so the window opens with a working app.
    let mut stamp = sources_stamp(dir);
    let first = loop {
        if let Some(b) = compile_ex(dir, true)? {
            break b;
        }
        println!("[plinth] fix the errors; waiting for changes...");
        while sources_stamp(dir) == stamp {
            std::thread::sleep(Duration::from_millis(300));
        }
        stamp = sources_stamp(dir);
    };
    println!("[plinth] running {} - edit app/ and save to reload; close the window to stop", first.config.name);
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let watch_dir = dir.to_path_buf();
    std::thread::spawn(move || {
        let mut last = stamp;
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let now = sources_stamp(&watch_dir);
            if now == last {
                continue;
            }
            last = now;
            println!("[plinth] change detected; building...");
            match compile_ex(&watch_dir, true) {
                Ok(Some(b)) => {
                    if tx.send(b.component).is_err() {
                        return;
                    }
                }
                Ok(None) => println!("[plinth] the app keeps the last good build"),
                Err(e) => eprintln!("error: {e:#}"),
            }
        }
    });
    let app = plinth_host_desktop::HostApp {
        component: first.component,
        title: first.config.name.clone(),
        accent: first.accent.unwrap_or_else(|| "teal".into()),
        app_id: first.config.id.clone(),
        version: first.config.version.clone(),
        capabilities: first.config.capabilities.iter().map(|c| c.name.clone()).collect(),
        assets: read_assets(dir),
        // `plinth dev` runs a project from source: its private space is
        // keyed by the app id alone, apart from any installed package.
        owner: plinth_host_desktop::Owner::Dev,
    };
    plinth_host_desktop::run(app, Some(rx))?;
    Ok(ExitCode::SUCCESS)
}

/// Adds `signature.json` to an existing `.plnt` (`plinth sign`,
/// `docs/HUB.md` §6.1).
fn sign_file(path: &Path) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mut pkg = Package::read(&bytes).map_err(|e| plinth_package::read_error(e, "not a valid .plnt package"))?;
    let identity = plinth_package::publisher::load()?;
    pkg.signature = Some(plinth_package::signature::sign(&pkg, &identity)?);
    std::fs::write(path, pkg.write()?).with_context(|| format!("write {}", path.display()))?;
    println!("signed {} as {} ({})", path.display(), identity.name, identity.key_id());
    Ok(())
}

/// `plinth publisher init|show` (`docs/HUB.md` §6.1, phase H1).
fn publisher_command(args: &[&str], raw: &[&str]) -> Result<ExitCode> {
    match args.first().copied() {
        Some("init") => {
            let name = raw
                .iter()
                .position(|a| *a == "--name")
                .and_then(|i| raw.get(i + 1))
                .copied()
                .or(args.get(1).copied())
                .context("usage: plinth publisher init [--name <publisher>]")?;
            let identity = plinth_package::publisher::init(name)?;
            println!("created publisher key at {}", plinth_package::publisher::publisher_dir().display());
            println!("{}  {}", identity.name, identity.key_id());
            Ok(ExitCode::SUCCESS)
        }
        Some("show") => {
            let identity = plinth_package::publisher::load()?;
            println!("{}  {}", identity.name, identity.key_id());
            Ok(ExitCode::SUCCESS)
        }
        Some(other) => bail!("unknown `plinth publisher {other}`; use init or show"),
        None => bail!("usage: plinth publisher init|show"),
    }
}

#[cfg(windows)]
fn register_scheme(exe: &Path) -> Result<()> {
    plinth_hub::os::register_scheme(&mut plinth_hub::os::CurrentUserRegistry, exe)
}

#[cfg(windows)]
fn unregister_scheme() -> Result<()> {
    plinth_hub::os::unregister_scheme(&mut plinth_hub::os::CurrentUserRegistry)
}

#[cfg(not(windows))]
fn register_scheme(_exe: &Path) -> Result<()> {
    bail!("`plinth hub register-scheme` is for Windows only (docs/HUB.md §10)")
}

#[cfg(not(windows))]
fn unregister_scheme() -> Result<()> {
    bail!("`plinth hub unregister-scheme` is for Windows only (docs/HUB.md §10)")
}

/// The app id of the Hub UI (`examples/hub`), which `plinth hub ui` runs
/// when no id is given.
const HUB_UI_APP_ID: &str = "dev.plinth.hub";

/// `plinth hub …` (`docs/HUB.md` §4.3, §9, §15 phase H0 part 4): a thin CLI
/// over `plinth-hub`, for testing the library, grants and consent flow
/// without the Hub UI (which comes in H3).
fn hub_command(args: &[&str], raw: &[&str]) -> Result<ExitCode> {
    let hub = plinth_hub::Hub::open_default()?;
    match args.first().copied() {
        Some("source") => {
            match args.get(1).copied() {
                Some("add") => {
                    let name = args.get(2).context("usage: plinth hub source add <name> <base>")?;
                    let base = args.get(3).context("usage: plinth hub source add <name> <base>")?;
                    hub.source_add(name, base)?;
                    println!("added source {name} ({base})");
                }
                Some("list") | None => {
                    for (name, base) in hub.sources()? {
                        // Header names only, never a value (`docs/HUB.md` §5.4).
                        let names: Vec<String> = hub.source_headers(&name)?.into_iter().map(|h| h.name).collect();
                        if names.is_empty() {
                            println!("{name}  {base}");
                        } else {
                            println!("{name}  {base}  [headers: {}]", names.join(", "));
                        }
                    }
                }
                Some("remove") => {
                    let name = args.get(2).context("usage: plinth hub source remove <name>")?;
                    // The token is in the keychain, so remove it first.
                    if !hub.source_headers(name)?.is_empty() {
                        plinth_host_desktop::source_headers::clear_headers(&hub, name, &mut *plinth_host_desktop::secrets::system_secrets())?;
                    }
                    hub.source_remove(name)?;
                    println!("removed source {name}");
                }
                Some("set-token") => {
                    // `docs/HUB.md` §5.4: a Cloudflare Access service token.
                    // Order: a flag, then CF_ACCESS_CLIENT_ID / _SECRET, then
                    // a line from stdin (id first, then secret).
                    let usage = "usage: plinth hub source set-token <name> [--client-id <id>] [--client-secret <secret>] (or CF_ACCESS_CLIENT_ID and CF_ACCESS_CLIENT_SECRET, or two lines on stdin)";
                    let name = args.get(2).context(usage)?;
                    let flag = |f: &str| raw.iter().position(|a| *a == f).and_then(|i| raw.get(i + 1)).map(|s| (*s).to_owned());
                    let read_line = |what: &str| -> Result<String> {
                        let mut line = String::new();
                        std::io::stdin().read_line(&mut line)?;
                        let line = line.trim().to_owned();
                        if line.is_empty() {
                            bail!("no {what}; {usage}");
                        }
                        Ok(line)
                    };
                    let id = match flag("--client-id").or_else(|| std::env::var("CF_ACCESS_CLIENT_ID").ok().filter(|v| !v.is_empty())) {
                        Some(v) => v,
                        None => read_line("client id")?,
                    };
                    let secret = match flag("--client-secret").or_else(|| std::env::var("CF_ACCESS_CLIENT_SECRET").ok().filter(|v| !v.is_empty())) {
                        Some(v) => v,
                        None => read_line("client secret")?,
                    };
                    plinth_host_desktop::source_headers::set_access_token(&hub, name, &id, &secret)?;
                    println!("saved the access token for source {name} in the system keychain");
                }
                Some("clear-token") => {
                    let name = args.get(2).context("usage: plinth hub source clear-token <name>")?;
                    plinth_host_desktop::source_headers::clear_headers(&hub, name, &mut *plinth_host_desktop::secrets::system_secrets())?;
                    println!("removed the access token of source {name}");
                }
                Some("trust-hub") => {
                    // `docs/HUB.md` §4.1: the user decides which key is a Hub
                    // key. This reads `hub.json` of the source and saves its
                    // keys only with `--yes`.
                    let usage = "usage: plinth hub source trust-hub <name> [--yes]";
                    let name = args.get(2).context(usage)?;
                    let sources = hub.sources()?;
                    let base = sources.get(*name).with_context(|| format!("no source named `{name}`; see `plinth hub source list`"))?;
                    let source = plinth_host_desktop::source_headers::hub_sources(&hub).open(base).with_context(|| format!("open source {name} ({base})"))?;
                    let config = source.hub_config().with_context(|| format!("read hub.json of source {name} ({base})"))?;
                    if config.trusted_keys.is_empty() {
                        bail!("hub.json of source {name} ({base}) lists no key");
                    }
                    for key in &config.trusted_keys {
                        plinth_hub::validate_hub_key_id(key).with_context(|| format!("hub.json of source {name} lists a bad key"))?;
                    }
                    println!("source {name} ({base}) lists these Hub keys in hub.json:");
                    for key in &config.trusted_keys {
                        println!("  {key}");
                    }
                    if !raw.contains(&"--yes") {
                        println!("nothing is saved");
                        eprintln!("[plinth] if you trust {base}, run `plinth hub source trust-hub {name} --yes`");
                        return Ok(ExitCode::FAILURE);
                    }
                    for key in &config.trusted_keys {
                        let new = hub.trust_hub_key(key)?;
                        println!("{} {key} as a Hub key", if new { "trusted" } else { "already trusted:" });
                    }
                }
                Some(other) => bail!("unknown `plinth hub source {other}`; use add, list, remove, set-token, clear-token or trust-hub"),
            }
            return Ok(ExitCode::SUCCESS);
        }
        Some("register-scheme") => {
            // `docs/HUB.md` §10: Windows runs `plinth hub open <link>` for a
            // plinth:// link. Per user (HKEY_CURRENT_USER), no administrator.
            // `plinthw` (next to this program) runs it with no console window.
            let exe = match raw.iter().position(|a| *a == "--exe").and_then(|i| raw.get(i + 1)) {
                Some(exe) => PathBuf::from(exe),
                None => plinth_hub::os::gui_launcher(&std::env::current_exe()?),
            };
            register_scheme(&exe)?;
            println!("registered plinth:// links to run {} hub open", exe.display());
            return Ok(ExitCode::SUCCESS);
        }
        Some("unregister-scheme") => {
            unregister_scheme()?;
            println!("removed the plinth:// registration");
            return Ok(ExitCode::SUCCESS);
        }
        Some("open") => {
            let url = args.get(1).context("usage: plinth hub open <plinth://app/<app id>>")?;
            let sources: std::sync::Arc<dyn plinth_hub::SourceClient> = std::sync::Arc::new(plinth_host_desktop::source_headers::hub_sources(&hub));
            let id = match plinth_hub::os::parse_link(url)? {
                plinth_hub::os::Link::Hub => HUB_UI_APP_ID.to_owned(),
                plinth_hub::os::Link::App { id, version } => {
                    // `docs/HUB.md` §5.3: an app that is not in the library
                    // comes from the user's sources (the latest version).
                    if hub.get(&id)?.is_none() {
                        if let Some(version) = &version {
                            eprintln!("[plinth] installing the latest version of {id}, not {version} (pinned links are not supported yet)");
                        }
                        plinth_hub::install_from_sources(&hub, sources.as_ref(), &id)?;
                        eprintln!("[plinth] installed {id}");
                    }
                    id
                }
            };
            plinth_host_desktop::init_logging();
            plinth_host_desktop::run_from_hub_with_sources(&hub, &id, Some(sources))?;
            return Ok(ExitCode::SUCCESS);
        }
        Some("shortcut") => {
            // `docs/HUB.md` §10: a `.lnk` with the app icon on the desktop or
            // in the Start menu; `--url` writes the older Internet Shortcut.
            let id = args.get(1).context("usage: plinth hub shortcut <app id> [--start-menu] [--dir <folder>] [--url]")?;
            let entry = hub.get(id)?.with_context(|| format!("{id} is not in the library"))?;
            let dir = match raw.iter().position(|a| *a == "--dir").and_then(|i| raw.get(i + 1)) {
                Some(dir) => PathBuf::from(dir),
                None if raw.contains(&"--start-menu") => {
                    plinth_hub::os::start_menu_dir().context("no Start menu folder on this platform; use --dir <folder>")?
                }
                None => plinth_hub::os::desktop_dir().context("no desktop folder on this platform; use --dir <folder>")?,
            };
            let exe = std::env::current_exe()?;
            if raw.contains(&"--url") {
                let path = plinth_hub::os::write_shortcut(&dir, id, &entry.name, &exe)?;
                println!("wrote {} (it opens plinth://app/{id}; run `plinth hub register-scheme` one time)", path.display());
                return Ok(ExitCode::SUCCESS);
            }
            let icon = match hub.write_app_icon(id) {
                Ok(icon) => icon,
                Err(e) => {
                    eprintln!("[plinth] the shortcut gets the Plinth icon: {e:#}");
                    None
                }
            };
            let launcher = plinth_hub::os::gui_launcher(&exe);
            let path = plinth_hub::os::write_link_shortcut(&dir, id, &entry.name, &launcher, icon.as_deref())?;
            println!("wrote {} (it runs {} {})", path.display(), launcher.display(), plinth_hub::os::shortcut_arguments(id));
            return Ok(ExitCode::SUCCESS);
        }
        Some("search") => {
            let text = args.get(1).context("usage: plinth hub search <text>")?;
            for (name, base) in hub.sources()? {
                let source = plinth_host_desktop::source_headers::hub_sources(&hub).open(&base).with_context(|| format!("open source {name} ({base})"))?;
                for app in source.search(text)? {
                    println!("{}  {}  {}  [{name}]", app.id, app.name, app.latest);
                }
            }
            return Ok(ExitCode::SUCCESS);
        }
        Some("install") => {
            let spec = args.get(1).context("usage: plinth hub install <id>[@version] [--source <name>]")?;
            let (id, version) = spec.split_once('@').map(|(i, v)| (i, Some(v.to_owned()))).unwrap_or((spec, None));
            let source_flag = raw.iter().position(|a| *a == "--source").and_then(|i| raw.get(i + 1)).copied();
            let (name, base) = resolve_source(&hub, id, source_flag)?;
            let source = plinth_host_desktop::source_headers::hub_sources(&hub).open(&base).with_context(|| format!("open source {name} ({base})"))?;
            let doc = source.app(id)?;
            let version = version.unwrap_or_else(|| doc.latest().map(|v| v.version.clone()).unwrap_or_default());
            if version.is_empty() {
                bail!("{id} has no installable version in source {name}");
            }
            let bytes = source.package(id, &version)?;
            let added = hub.add_package(&bytes)?;
            hub.set_registry(&added, &name, &base)?;
            println!("installed {added}@{version} from {name}");
            return Ok(ExitCode::SUCCESS);
        }
        Some("update") => {
            // `docs/HUB.md` §9.2: the same code path as the Hub UI's
            // `update` (`plinth_hub::apply_update`).
            let only = args.get(1).copied();
            let client = plinth_host_desktop::source_headers::hub_sources(&hub);
            for entry in hub.list()? {
                if let Some(only) = only
                    && entry.id != only
                {
                    continue;
                }
                if entry.registry.is_none() {
                    continue;
                }
                let Some(updated) = plinth_hub::apply_update(&hub, &client, &entry.id)? else { continue };
                println!("updated {} to {} (from {})", updated.id, updated.to, updated.source);
                if !updated.new_capabilities.is_empty() {
                    println!("  new capabilities: {}", updated.new_capabilities.join(", "));
                    println!("  note: `plinth hub run {}` will ask for them (medium/high risk only)", entry.id);
                }
                if let Some(pinned) = &entry.pinned {
                    println!("  note: {} is pinned to {pinned}; `plinth hub pin {} --latest` runs the new version", entry.id, entry.id);
                }
            }
            return Ok(ExitCode::SUCCESS);
        }
        _ => {}
    }
    match args.first().copied() {
        Some("add") => {
            let file = args.get(1).context("usage: plinth hub add <app.plnt>")?;
            let bytes = std::fs::read(file).with_context(|| format!("read {file}"))?;
            let id = hub.add_package(&bytes)?;
            println!("added {id}");
        }
        Some("list") | None => {
            let apps = hub.list()?;
            if apps.is_empty() {
                println!("the library is empty");
            }
            for app in apps {
                let version = app.active_version().map(|v| v.version.as_str()).unwrap_or("-");
                let pin = if app.pinned.is_some() { " (pinned)" } else { "" };
                let signer = match app.active_version().and_then(|v| v.signer.as_deref()) {
                    Some(key) => format!("signed by {key}"),
                    None => "unverified publisher".to_owned(),
                };
                println!("{}  {}  {}{}  [{signer}]", app.id, app.name, version, pin);
            }
        }
        Some(cmd @ ("run" | "ui")) => {
            // `plinth hub ui` runs the Hub UI app (`docs/HUB.md` §4.1,
            // `examples/hub`); `run` needs an id. Either way, an app that
            // the Hub UI launches opens in a new window of this process.
            let id = match (cmd, args.get(1)) {
                (_, Some(id)) => *id,
                ("ui", None) => HUB_UI_APP_ID,
                _ => bail!("usage: plinth hub run <app id>"),
            };
            plinth_host_desktop::init_logging();
            let sources: std::sync::Arc<dyn plinth_hub::SourceClient> = std::sync::Arc::new(plinth_host_desktop::source_headers::hub_sources(&hub));
            plinth_host_desktop::run_from_hub_with_sources(&hub, id, Some(sources))?;
        }
        Some("remove") => {
            let id = args.get(1).context("usage: plinth hub remove <app id>")?;
            hub.remove(id)?;
            println!("removed {id}");
        }
        Some("uninstall") => {
            // `docs/STORAGE.md` S13: the package and, unless `--keep-data`, all data.
            let id = args.get(1).context("usage: plinth hub uninstall <app id> [--keep-data]")?;
            let keep_data = raw.contains(&"--keep-data");
            let data_dir = plinth_runner_wasmtime::kv::data_dir();
            let report = hub.uninstall(id, &data_dir, keep_data, &mut |app| {
                plinth_host_desktop::secrets::remove_app_secrets(app).map_err(|e| anyhow::anyhow!("{e:?}"))
            })?;
            if keep_data {
                println!("uninstalled {id}; its data, grants and secrets stay");
            } else {
                println!(
                    "uninstalled {id}; deleted {} of data, {} grant record(s) and the secrets",
                    format_bytes(report.deleted_bytes),
                    report.grants_removed
                );
            }
        }
        Some("export") => {
            // `docs/STORAGE.md` S13. The secrets are never exported.
            let usage = "usage: plinth hub export <app id> <out.zip>";
            let id = args.get(1).context(usage)?;
            let out = args.get(2).context(usage)?;
            let data_dir = plinth_runner_wasmtime::kv::data_dir();
            let report = hub.export_data(id, &data_dir, Path::new(out))?;
            println!(
                "exported {id} to {out}: {} file(s), {} database(s), {} key(s) of kv ({})",
                report.files,
                report.databases,
                report.kv_entries.unwrap_or(0),
                format_bytes(report.bytes)
            );
            println!("the secrets of the app are not in the export");
        }
        Some("grants") => {
            let id = args.get(1).context("usage: plinth hub grants <app id> [allow|refuse <capability>]")?;
            match args.get(2).copied() {
                None => {
                    let entry = hub.get(id)?.with_context(|| format!("{id} is not in the library"))?;
                    let declared = entry.active_version().map(|v| v.capabilities.clone()).unwrap_or_default();
                    for status in hub.capability_report(id, &declared)? {
                        let risk = match status.risk {
                            plinth_link::capabilities::Risk::None => "none",
                            plinth_link::capabilities::Risk::Low => "low",
                            plinth_link::capabilities::Risk::Medium => "medium",
                            plinth_link::capabilities::Risk::High => "high",
                        };
                        let by = if status.by_default { "default" } else { "user" };
                        match status.decision {
                            Some(d) => println!("{}  risk={risk}  {:?} by {by}", status.name, d),
                            None => println!("{}  risk={risk}  not decided", status.name),
                        }
                    }
                }
                Some(action @ ("allow" | "refuse")) => {
                    let capability = args.get(3).context("usage: plinth hub grants <app id> allow|refuse <capability>")?;
                    let entry = hub.get(id)?.with_context(|| format!("{id} is not in the library"))?;
                    let version = entry.active_version().map(|v| v.version.clone()).unwrap_or_default();
                    let decision = if action == "allow" { plinth_hub::Decision::Allowed } else { plinth_hub::Decision::Refused };
                    hub.set_grant(id, capability, decision, &version)?;
                    let verb = if action == "allow" { "allowed" } else { "refused" };
                    println!("{verb} {capability} for {id}");
                }
                Some(other) => bail!("unknown `plinth hub grants {other}`; use allow or refuse"),
            }
        }
        Some("block") => {
            let id = args.get(1).context("usage: plinth hub block <app id>")?;
            hub.block_app(id)?;
            println!("blocked {id}");
        }
        Some("unblock") => {
            let id = args.get(1).context("usage: plinth hub unblock <app id>")?;
            hub.unblock_app(id)?;
            println!("unblocked {id}");
        }
        Some("pin") => {
            // `docs/HUB.md` §9.2: run one installed version, not the newest.
            let usage = "usage: plinth hub pin <app id> <version> | plinth hub pin <app id> --latest";
            let id = args.get(1).context(usage)?;
            if raw.contains(&"--latest") {
                hub.pin(id, None)?;
                println!("{id} runs the newest version again");
            } else {
                let version = args.get(2).context(usage)?;
                hub.pin(id, Some((*version).to_owned()))?;
                println!("pinned {id} to {version}");
            }
        }
        Some("block-publisher") => {
            let key = args.get(1).context("usage: plinth hub block-publisher <key id>")?;
            hub.block_publisher(key)?;
            println!("blocked publisher {key} (and all its apps)");
        }
        Some("unblock-publisher") => {
            let key = args.get(1).context("usage: plinth hub unblock-publisher <key id>")?;
            hub.unblock_publisher(key)?;
            println!("unblocked publisher {key}");
        }
        Some("groups") => match args.get(1).copied() {
            None => {
                for group in hub.groups()? {
                    println!("{group}");
                }
            }
            Some("create") => {
                let name = args.get(2).context("usage: plinth hub groups create <name>")?;
                hub.create_group(name)?;
                println!("created group {name}");
            }
            Some("add") => {
                let id = args.get(2).context("usage: plinth hub groups add <app id> <name>")?;
                let name = args.get(3).context("usage: plinth hub groups add <app id> <name>")?;
                hub.add_to_group(id, name)?;
                println!("added {id} to {name}");
            }
            Some(other) => bail!("unknown `plinth hub groups {other}`; use create or add"),
        },
        Some("policy") => match args.get(1).copied() {
            Some("deny") => {
                let capability = args.get(2).context("usage: plinth hub policy deny <capability>")?;
                hub.policy_deny(capability)?;
                println!("denied {capability} for all apps");
            }
            Some("allow") => {
                let capability = args.get(2).context("usage: plinth hub policy allow <capability>")?;
                hub.policy_allow(capability)?;
                println!("allowed {capability} again");
            }
            Some("quota") => {
                let usage = "usage: plinth hub policy quota <MiB> | default";
                match args.get(2).copied().context(usage)? {
                    "default" => {
                        hub.set_data_quota_mib(None)?;
                        println!("the data quota of an app is {} again", format_bytes(plinth_hub::DEFAULT_DATA_QUOTA));
                    }
                    mib => {
                        let mib: u64 = mib.parse().with_context(|| usage.to_owned())?;
                        hub.set_data_quota_mib(Some(mib))?;
                        println!("the data quota of an app is {mib} MiB for each store (private files, kv, sql)");
                    }
                }
            }
            Some("trust-hub-key") => {
                let usage = "usage: plinth hub policy trust-hub-key <key id>";
                let key = args.get(2).context(usage)?;
                if hub.trust_hub_key(key)? {
                    println!("trusted {key} as a Hub key: a package signed by it may use hub.manage");
                } else {
                    println!("{key} is already a trusted Hub key");
                }
            }
            Some("untrust-hub-key") => {
                let key = args.get(2).context("usage: plinth hub policy untrust-hub-key <key id>")?;
                if hub.untrust_hub_key(key)? {
                    println!("{key} is not a trusted Hub key any more");
                } else {
                    println!("{key} was not a saved Hub key");
                }
            }
            Some("show") | None => {
                println!("data quota of an app: {} for each store", format_bytes(hub.data_quota()));
                let saved = hub.saved_hub_keys()?;
                let all = plinth_hub::trusted_hub_keys_in(hub.dir());
                if all.is_empty() {
                    println!("no Hub key is trusted (`plinth hub policy trust-hub-key <key id>`)");
                }
                for key in &all {
                    let from = if saved.contains(key) { "saved" } else { "PLINTH_HUB_TRUSTED_KEYS" };
                    println!("{key}  trusted Hub key ({from})");
                }
                let denied = hub.policy_denied()?;
                if denied.is_empty() {
                    println!("no capability is globally denied");
                } else {
                    for capability in denied {
                        println!("{capability}  denied for all apps");
                    }
                }
            }
            Some(other) => bail!("unknown `plinth hub policy {other}`; use deny, allow, quota, trust-hub-key, untrust-hub-key or show"),
        },
        Some(other) => bail!("unknown `plinth hub {other}`; see `plinth --help`"),
    }
    Ok(ExitCode::SUCCESS)
}

/// `12.3 MiB`, `512.0 KiB` or `7 bytes`.
fn format_bytes(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MiB", n as f64 / (1 << 20) as f64)
    } else if n >= 1 << 10 {
        format!("{:.1} KiB", n as f64 / (1 << 10) as f64)
    } else {
        format!("{n} bytes")
    }
}

/// Picks the registry source for `hub install`/`update`: the `--source`
/// flag if given, else the only configured source, else an error naming
/// the choices (`docs/REGISTRY.md` §9).
fn resolve_source(hub: &plinth_hub::Hub, _id: &str, flag: Option<&str>) -> Result<(String, String)> {
    let sources = hub.sources()?;
    if let Some(name) = flag {
        let base = sources.get(name).with_context(|| format!("no source named `{name}`; see `plinth hub source list`"))?;
        return Ok((name.to_owned(), base.clone()));
    }
    match sources.len() {
        0 => bail!("no registry sources are configured; add one with `plinth hub source add <name> <base>`"),
        1 => {
            let (name, base) = sources.into_iter().next().unwrap();
            Ok((name, base))
        }
        _ => bail!("more than one registry source is configured; pick one with --source <name> (`plinth hub source list`)"),
    }
}

/// `plinth registry build|serve` (`docs/REGISTRY.md` §8).
fn registry_command(args: &[&str], raw: &[&str]) -> Result<ExitCode> {
    match args.first().copied() {
        Some("build") => {
            let folder = args.get(1).context("usage: plinth registry build <folder> [--with-core] [--hub-trusted-key <key id>]")?;
            // `--hub-trusted-key <id>` (more than one, or comma-separated):
            // writes hub.json for the web App Hub (docs/web-hub.md).
            let mut keys = Vec::new();
            for (i, a) in raw.iter().enumerate() {
                if *a == "--hub-trusted-key" {
                    let v = raw.get(i + 1).context("--hub-trusted-key needs a key id (ed25519:…)")?;
                    keys.extend(v.split(',').map(str::trim).filter(|k| !k.is_empty()).map(str::to_owned));
                }
            }
            registry_cmd::build(Path::new(folder), raw.iter().any(|a| *a == "--with-core"), keys)?;
            Ok(ExitCode::SUCCESS)
        }
        Some("serve") => {
            let folder = args.get(1).context("usage: plinth registry serve <folder> [--port N] [--web]")?;
            let port: u16 = match raw.iter().position(|a| *a == "--port") {
                Some(i) => raw.get(i + 1).and_then(|p| p.parse().ok()).context("--port needs a number from 0 to 65535")?,
                None => 8080,
            };
            registry_cmd::serve(Path::new(folder), port, raw.iter().any(|a| *a == "--web"))?;
            Ok(ExitCode::SUCCESS)
        }
        Some(other) => bail!("unknown `plinth registry {other}`; use build or serve"),
        None => bail!("usage: plinth registry build|serve <folder>"),
    }
}

/// `plinth feed add <feed-dir> <app.plnt> [--channel <name>] [--with-core]`
/// (`docs/UPDATES.md` §8 step 3).
fn feed_command(args: &[&str], raw: &[&str]) -> Result<ExitCode> {
    match args.first().copied() {
        Some("add") => {
            let feed_dir = args.get(1).context("usage: plinth feed add <feed-dir> <app.plnt> [--channel <name>] [--with-core]")?;
            let package = args.get(2).context("usage: plinth feed add <feed-dir> <app.plnt> [--channel <name>] [--with-core]")?;
            let channel = raw.iter().position(|a| *a == "--channel").and_then(|i| raw.get(i + 1)).copied().unwrap_or("stable");
            feed_cmd::add(Path::new(feed_dir), Path::new(package), channel, raw.iter().any(|a| *a == "--with-core"))?;
            Ok(ExitCode::SUCCESS)
        }
        Some(other) => bail!("unknown `plinth feed {other}`; use add"),
        None => bail!("usage: plinth feed add <feed-dir> <app.plnt>"),
    }
}

#[cfg(test)]
mod typings_tests {
    use super::{TYPINGS, TYPINGS_LEFT_OUT};

    #[test]
    fn every_std_typing_is_written_or_left_out_with_a_reason() {
        let std_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../std");
        let mut files: Vec<String> = std::fs::read_dir(&std_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".d.ts"))
            .collect();
        files.sort();
        assert!(!files.is_empty(), "no typings in {}", std_dir.display());
        for f in &files {
            let written = TYPINGS.iter().any(|(n, _)| n == f);
            let left_out = TYPINGS_LEFT_OUT.iter().any(|(n, _)| n == f);
            assert!(written || left_out, "std/{f} is not in TYPINGS (crates/plinth-cli/src/main.rs); add it, or add it to TYPINGS_LEFT_OUT with a reason");
            assert!(!(written && left_out), "std/{f} is in TYPINGS and in TYPINGS_LEFT_OUT");
        }
        for (n, _) in TYPINGS.iter().chain(TYPINGS_LEFT_OUT) {
            assert!(files.iter().any(|f| f == n), "{n} is listed, but std/{n} does not exist");
        }
        for (n, why) in TYPINGS_LEFT_OUT {
            assert!(!why.trim().is_empty(), "{n} is left out with no reason");
        }
    }
}
