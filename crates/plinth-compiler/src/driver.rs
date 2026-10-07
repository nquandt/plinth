//! Loads a project, resolves its modules in the closed world (SPEC.md §4.1),
//! and runs the compiler phases.

use crate::ast::Item;
use crate::check::{self, ModRef, ModuleSrc, StdModule};
use crate::diag::{Diagnostic, FileId, Severity, Sources, Span, code};
use crate::tir::Program;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The entry module of every app.
pub const ENTRY: &str = "app/main.tsx";

/// Where the compiler reads source files. Paths are relative to the project
/// root and use `/`.
pub trait FileSystem {
    fn read(&self, path: &str) -> Option<String>;
    /// File names under `assets/` (SPEC.md §10.1), used to check
    /// `<Image src>`. The default is empty.
    fn list_assets(&self) -> Vec<String> {
        Vec::new()
    }
}

pub struct DiskFs {
    pub root: PathBuf,
}

impl FileSystem for DiskFs {
    fn read(&self, path: &str) -> Option<String> {
        let p = self.root.join(path);
        // Only regular files inside the root (no symlinks out of the app).
        let meta = std::fs::symlink_metadata(&p).ok()?;
        if !meta.is_file() {
            return None;
        }
        std::fs::read_to_string(p).ok()
    }

    fn list_assets(&self) -> Vec<String> {
        let dir = self.root.join("assets");
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&dir) else { return out };
        for e in rd.flatten() {
            if let Ok(meta) = e.metadata() {
                if meta.is_file() {
                    if let Some(name) = e.file_name().to_str() {
                        out.push(name.to_owned());
                    }
                }
            }
        }
        out
    }
}

/// An in-memory project, for tests and tools.
#[derive(Default)]
pub struct MemFs(pub HashMap<String, String>);

impl MemFs {
    pub fn with(mut self, path: &str, text: &str) -> Self {
        self.0.insert(path.to_owned(), text.to_owned());
        self
    }
}

impl FileSystem for MemFs {
    fn read(&self, path: &str) -> Option<String> {
        self.0.get(path).cloned()
    }

    /// Any entry whose path starts with `assets/` counts as an asset; the
    /// `with` text is unused but keeps a single map simple for tests.
    fn list_assets(&self) -> Vec<String> {
        self.0.keys().filter_map(|p| p.strip_prefix("assets/")).map(str::to_owned).collect()
    }
}

pub struct Frontend {
    pub sources: Sources,
    pub diags: Vec<Diagnostic>,
    pub program: Option<Program>,
}

impl Frontend {
    pub fn has_errors(&self) -> bool {
        self.diags.iter().any(|d| d.severity == Severity::Error)
    }
}

struct Loaded {
    path: String,
    file: FileId,
    ast: crate::ast::Module,
    /// One entry per import item: the resolved target.
    imports: Vec<Option<ModRef>>,
    /// `(dependency index, span of the import)`.
    deps: Vec<(usize, Span)>,
}

/// Parses and checks a project with no declared capabilities (SPEC.md
/// §11): any host API call that needs one is reported.
pub fn frontend(fs: &dyn FileSystem) -> Frontend {
    frontend_with_capabilities(fs, &[])
}

/// Parses and checks a project. The program is `None` when there are
/// errors. `capabilities` are the capability names from `plinth.toml`
/// (SPEC.md §11), used to check host API calls.
pub fn frontend_with_capabilities(fs: &dyn FileSystem, capabilities: &[String]) -> Frontend {
    crate::with_stack(|| frontend_impl(fs, capabilities))
}

fn frontend_impl(fs: &dyn FileSystem, capabilities: &[String]) -> Frontend {
    let mut sources = Sources::default();
    let mut diags = Vec::new();
    let Some(text) = fs.read(ENTRY) else {
        let file = sources.add(ENTRY.into(), String::new());
        diags.push(
            Diagnostic::error(code::MODULE_NOT_FOUND, Span::new(file, 0, 0), "the app has no app/main.tsx")
                .help("run `plinth new` to create a project"),
        );
        return Frontend { sources, diags, program: None };
    };

    // Load all modules, breadth first.
    let mut loaded: Vec<Loaded> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut queue = vec![(ENTRY.to_owned(), text)];
    while let Some((path, text)) = queue.pop() {
        let file = sources.add(path.clone(), text);
        let is_tsx = path.ends_with(".tsx");
        let ast = crate::parse::parse(file, sources.text(file), is_tsx, &mut diags);
        index.insert(path.clone(), loaded.len());
        loaded.push(Loaded { path, file, ast, imports: Vec::new(), deps: Vec::new() });
        let me = loaded.len() - 1;
        let imports: Vec<(String, Span)> = loaded[me]
            .ast
            .items
            .iter()
            .filter_map(|i| if let Item::Import(imp) = i { Some((imp.source.clone(), imp.source_span)) } else { None })
            .collect();
        for (spec, span) in imports {
            let target = match resolve(fs, &loaded[me].path, &spec) {
                Ok(Target::Std(m)) => Some(ModRef::Std(m)),
                Ok(Target::File(p, text)) => {
                    let idx = match index.get(&p) {
                        Some(i) => *i,
                        None => {
                            // Reserve the index now; the file loads later.
                            let pending = queue.iter().position(|(q, _)| *q == p);
                            if pending.is_none() {
                                queue.insert(0, (p.clone(), text));
                            }
                            usize::MAX
                        }
                    };
                    loaded[me].deps.push((idx, span));
                    if idx == usize::MAX {
                        // Patched after all modules have loaded.
                        loaded[me].imports.push(Some(ModRef::User(usize::MAX)));
                        PENDING.with(|pd| pd.borrow_mut().push((me, loaded[me].imports.len() - 1, p, span)));
                        continue;
                    }
                    Some(ModRef::User(idx))
                }
                Err(d) => {
                    diags.push(d(span));
                    None
                }
            };
            loaded[me].imports.push(target);
        }
    }
    // Patch the imports of modules that loaded after their importers.
    for (m, slot, path, span) in PENDING.with(|pd| std::mem::take(&mut *pd.borrow_mut())) {
        let idx = index[&path];
        loaded[m].imports[slot] = Some(ModRef::User(idx));
        for d in &mut loaded[m].deps {
            if d.0 == usize::MAX && d.1 == span {
                d.0 = idx;
                break;
            }
        }
    }

    // Order: dependencies first. A cycle is an error.
    let mut order = Vec::new();
    let mut state = vec![0u8; loaded.len()]; // 0 new, 1 visiting, 2 done
    fn visit(i: usize, loaded: &[Loaded], state: &mut [u8], order: &mut Vec<usize>, diags: &mut Vec<Diagnostic>) {
        state[i] = 1;
        for &(d, span) in &loaded[i].deps {
            match state[d] {
                0 => visit(d, loaded, state, order, diags),
                1 => diags.push(
                    Diagnostic::error(code::IMPORT_CYCLE, span, format!("import cycle: `{}` imports a module that imports it", loaded[i].path))
                        .help("move the shared code into a separate module"),
                ),
                _ => {}
            }
        }
        state[i] = 2;
        order.push(i);
    }
    visit(0, &loaded, &mut state, &mut order, &mut diags);

    let has_syntax_errors = diags.iter().any(|d| d.severity == Severity::Error);
    if has_syntax_errors {
        return Frontend { sources, diags, program: None };
    }

    // Renumber to the checking order.
    let mut new_index = vec![0usize; loaded.len()];
    for (pos, &i) in order.iter().enumerate() {
        new_index[i] = pos;
    }
    let mut by_old: Vec<Option<Loaded>> = loaded.into_iter().map(Some).collect();
    let modules: Vec<ModuleSrc> = order
        .iter()
        .map(|&i| {
            let l = by_old[i].take().unwrap();
            let imports = l
                .imports
                .into_iter()
                .map(|t| match t {
                    Some(ModRef::User(d)) => Some(ModRef::User(new_index[d])),
                    other => other,
                })
                .collect();
            ModuleSrc { file: l.file, ast: l.ast, imports }
        })
        .collect();
    let main = new_index[0];
    let assets = fs.list_assets();
    let program = check::check_ex(&modules, main, &mut diags, capabilities, &assets);
    let ok = !diags.iter().any(|d| d.severity == Severity::Error);
    Frontend { sources, diags, program: ok.then_some(program) }
}

thread_local! {
    static PENDING: std::cell::RefCell<Vec<(usize, usize, String, Span)>> = const { std::cell::RefCell::new(Vec::new()) };
}

enum Target {
    Std(StdModule),
    File(String, String),
}

type DiagFn = Box<dyn Fn(Span) -> Diagnostic>;

fn resolve(fs: &dyn FileSystem, from: &str, spec: &str) -> Result<Target, DiagFn> {
    if let Some(name) = spec.strip_prefix("plinth:") {
        return match StdModule::from_specifier(spec) {
            Some(m) => Ok(Target::Std(m)),
            None => {
                let name = name.to_owned();
                Err(Box::new(move |span| {
                    Diagnostic::error(code::UNKNOWN_STD_MODULE, span, format!("`plinth:{name}` is not a Plinth module yet"))
                        .help("available modules: plinth:ui, plinth:core, plinth:time, plinth:store, plinth:clipboard, plinth:dialog, plinth:hub")
                }))
            }
        };
    }
    if spec.starts_with("hub:") {
        return Err(Box::new(|span| {
            Diagnostic::error(code::BARE_IMPORT, span, "hub libraries are not supported yet").help("copy the code into app/")
        }));
    }
    if !spec.starts_with("./") && !spec.starts_with("../") {
        let spec = spec.to_owned();
        return Err(Box::new(move |span| {
            Diagnostic::error(code::BARE_IMPORT, span, format!("cannot import `{spec}`: Plinth apps cannot use npm packages"))
                .help("import `plinth:*` modules or relative files (`./x`); see SPEC.md §4.1")
        }));
    }
    let dir = Path::new(from).parent().unwrap_or(Path::new(""));
    let Some(base) = normalize(&dir.join(spec)) else {
        return Err(Box::new(|span| Diagnostic::error(code::MODULE_NOT_FOUND, span, "the import leaves the project")));
    };
    if !base.starts_with("app/") {
        return Err(Box::new(|span| {
            Diagnostic::error(code::MODULE_NOT_FOUND, span, "imports must stay inside app/").help("move the file into app/")
        }));
    }
    for cand in [base.clone(), format!("{base}.tsx"), format!("{base}.ts"), format!("{base}/index.tsx"), format!("{base}/index.ts")] {
        if !(cand.ends_with(".ts") || cand.ends_with(".tsx")) {
            continue;
        }
        if let Some(text) = fs.read(&cand) {
            return Ok(Target::File(cand, text));
        }
    }
    let spec = spec.to_owned();
    Err(Box::new(move |span| Diagnostic::error(code::MODULE_NOT_FOUND, span, format!("cannot find module `{spec}`"))))
}

/// Joins `..` and `.` parts. `None` if the path climbs above the root.
fn normalize(p: &Path) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for c in p.components() {
        match c {
            std::path::Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            std::path::Component::ParentDir => {
                parts.pop()?;
            }
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}
