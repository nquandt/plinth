//! The `plinth` command-line tool (SPEC.md §13).

use anyhow::{Context as _, Result, bail};
use plinth_compiler::driver::{DiskFs, Frontend};
use plinth_package::{Package, ProjectConfig};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

mod dev_tools;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
plinth - build cross-platform apps from Plinth TS

usage:
  plinth new <dir>                 make a new app (hello world)
  plinth dev [dir]                 run the app; reload it when a file changes
  plinth check [dir] [--json] [--watch]
                                   type-check, with no build
  plinth build [dir] [--out <file>]
                                   make dist/<name>.plnt
  plinth run <app.plnt | app.wasm> run a package
  plinth validate <app.plnt | app.wasm>
                                   check a package and its Wasm imports

The project directory defaults to the current directory.";

/// The typings that `plinth` writes to `.plinth/types` (SPEC.md §4.7).
const TYPINGS: &[(&str, &str)] = &[
    ("lib.d.ts", include_str!("../../../std/lib.d.ts")),
    ("ui.d.ts", include_str!("../../../std/ui.d.ts")),
    ("core.d.ts", include_str!("../../../std/core.d.ts")),
];

fn main() -> ExitCode {
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
            let out = args.iter().position(|a| *a == "--out").and_then(|i| args.get(i + 1)).map(PathBuf::from);
            let project = positional.get(1).filter(|p| Some(**p) != out.as_ref().and_then(|o| o.to_str())).map(PathBuf::from);
            Ok(if build(&project.unwrap_or_else(|| PathBuf::from(".")), out)? { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Some("dev") => dev(&dir(1)),
        Some("run") => {
            let file = positional.get(1).context("usage: plinth run <app.plnt | app.wasm>")?;
            plinth_host_desktop::init_logging();
            plinth_host_desktop::run(plinth_host_desktop::HostApp::load(Path::new(file))?, None)?;
            Ok(ExitCode::SUCCESS)
        }
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
    write("tsconfig.json", include_str!("../templates/tsconfig.json"))?;
    write(".gitignore", include_str!("../templates/gitignore"))?;
    write("README.md", include_str!("../templates/README.md"))?;
    write(
        "plinth.toml",
        &format!("# The app metadata (SPEC.md §10.2).\nid = \"com.example.{id_part}\"\nname = \"{name}\"\nversion = \"0.1.0\"\npublisher = \"example\"\n"),
    )?;
    write(
        "package.json",
        &format!(
            "{{\n  \"name\": \"{slug}\",\n  \"version\": \"0.1.0\",\n  \"private\": true,\n  \"scripts\": {{\n    \"dev\": \"plinth dev\",\n    \"check\": \"plinth check\",\n    \"build\": \"plinth build\"\n  }},\n  \"devDependencies\": {{\n    \"plinth\": \"^{VERSION}\"\n  }}\n}}\n"
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
    ensure_project(dir)?;
    let front = plinth_compiler::driver::frontend(&DiskFs { root: dir.to_path_buf() });
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
    component: Vec<u8>,
    accent: Option<String>,
}

fn compile(dir: &Path) -> Result<Option<Built>> {
    let config = ensure_project(dir)?;
    let (front, artifact) = plinth_compiler::compile(&DiskFs { root: dir.to_path_buf() })?;
    let (e, w) = print_diags(&front);
    match artifact {
        Some(a) => {
            if w > 0 {
                eprintln!("{}", summary(e, w));
            }
            Ok(Some(Built { config, component: a.component, accent: a.accent }))
        }
        None => {
            eprintln!("{}", summary(e, w));
            Ok(None)
        }
    }
}

fn build(dir: &Path, out: Option<PathBuf>) -> Result<bool> {
    let started = std::time::Instant::now();
    let Some(b) = compile(dir)? else { return Ok(false) };
    let manifest = b.config.manifest(plinth_protocol::UI_API_VERSION, b.accent, &b.component);
    let pkg = Package { manifest, component: b.component, assets: Vec::new() };
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
        "built {} in {} ms (component {} KiB, package {} KiB)",
        out.display(),
        started.elapsed().as_millis(),
        pkg.component.len().div_ceil(1024),
        bytes.len().div_ceil(1024)
    );
    Ok(true)
}

// -- dev -----------------------------------------------------------------------

fn dev(dir: &Path) -> Result<ExitCode> {
    plinth_host_desktop::init_logging();
    println!("[plinth] dev: {}", dir.display());
    // Wait for the first good build, so the window opens with a working app.
    let mut stamp = sources_stamp(dir);
    let first = loop {
        if let Some(b) = compile(dir)? {
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
            match compile(&watch_dir) {
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
        title: first.config.name,
        accent: first.accent.unwrap_or_else(|| "teal".into()),
    };
    plinth_host_desktop::run(app, Some(rx))?;
    Ok(ExitCode::SUCCESS)
}
