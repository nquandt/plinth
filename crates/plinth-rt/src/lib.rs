//! `plinth-rt`: the runtime library that the compiler links into each app
//! artifact (SPEC.md §5.1 step 8).
//!
//! # ABI
//!
//! The compiler appends the app code to this module (see `plinth-compiler`'s
//! linker). The app code calls the exported `__plinth_rt_*` functions below.
//! The runtime calls app code only through **callables**: a pair of a thunk
//! (a function table index with the type `(env: i32) -> ()`) and an
//! environment (a closure object on the heap). A thunk reads its argument
//! with `arg_*` and gives its result with `ret_*`.
//!
//! At instantiation, the app's start function calls `set_main` with the
//! thunk that runs the module initializers and builds the screens.
//!
//! Values: `number` is f64; `boolean`, `int`, enums and node ids are i32;
//! every heap reference (string, array, object, closure) is an i32 address,
//! and `null` is 0. Signal and computed handles are i32.
//!
//! On wasm32 the crate is `no_std`: `core::fmt`, the float formatting of
//! `core` and the Unicode tables would make every app much larger (SPEC.md
//! §5.5). Host builds use `std`, for the unit tests.

#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

/// The core version, `MAJOR.MINOR` (SPEC.md §10.4). A host reads it from the
/// custom section `plinth-core` and picks a core for each app by it. Inside
/// one major version a core only adds functions; each addition increments
/// the minor version. Keep it equal to `CORE_MAJOR`/`CORE_MINOR` in the
/// compiler's `rt_abi.rs` (a compiler test checks it).
#[cfg(target_arch = "wasm32")]
#[used]
#[unsafe(link_section = "plinth-core")]
static CORE_VERSION: [u8; 3] = *b"1.1";

#[cfg(target_arch = "wasm32")]
mod allocator;
pub mod arrays;
mod global;
pub mod gc;
mod host;
mod reactive;
pub mod strings;
mod ui;

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;
use global::GlobalCell;
use plinth_protocol::decode_events;

mod bindings {
    wit_bindgen::generate!({
        path: "../../wit/plinth",
        world: "app",
    });
}

/// A value that crosses between the runtime and app code.
#[derive(Clone, Copy, Debug, Default)]
pub enum Val {
    #[default]
    None,
    F64(f64),
    I32(i32),
    Ref(u32),
}

impl Val {
    /// Identity: the same bits for numbers, the same address for references.
    pub fn same(&self, other: &Val) -> bool {
        match (self, other) {
            (Val::None, Val::None) => true,
            (Val::F64(a), Val::F64(b)) => a.to_bits() == b.to_bits(),
            (Val::I32(a), Val::I32(b)) => a == b,
            (Val::Ref(a), Val::Ref(b)) => a == b,
            _ => false,
        }
    }

    fn f64(self) -> f64 {
        match self {
            Val::F64(v) => v,
            Val::I32(v) => v as f64,
            _ => 0.0,
        }
    }

    fn i32(self) -> i32 {
        match self {
            Val::I32(v) => v,
            Val::Ref(p) => p as i32,
            Val::F64(v) => v as i32,
            Val::None => 0,
        }
    }
}

/// A thunk and its environment.
#[derive(Clone, Copy, Debug)]
pub struct Callable {
    pub thunk: u32,
    pub env: u32,
}

static ARG: GlobalCell<Val> = GlobalCell::new(Val::None);
static RESULT: GlobalCell<Val> = GlobalCell::new(Val::None);
static MAIN: GlobalCell<Option<u32>> = GlobalCell::new(None);

/// Calls app code. The thunk reads `arg` at entry and sets the result last,
/// so nested calls do not disturb the registers.
pub fn invoke(c: Callable, arg: Val) -> Val {
    ARG.set(arg);
    RESULT.set(Val::None);
    call_thunk(c.thunk, c.env);
    RESULT.take()
}

#[cfg(target_arch = "wasm32")]
fn call_thunk(thunk: u32, env: u32) {
    // On wasm32 a function pointer is an index into the function table.
    let f: extern "C" fn(i32) = unsafe { core::mem::transmute(thunk as usize) };
    f(env as i32);
}

#[cfg(not(target_arch = "wasm32"))]
fn call_thunk(_thunk: u32, _env: u32) {
    unreachable!("app code runs only on wasm32");
}

/// Logs `msg` and stops the guest. The host shows "This app stopped".
pub fn trap(msg: &str) -> ! {
    let mut line = String::from("trap: ");
    line.push_str(msg);
    log(&line);
    #[cfg(target_arch = "wasm32")]
    core::arch::wasm32::unreachable();
    #[cfg(not(target_arch = "wasm32"))]
    panic!("{msg}");
}

fn log(msg: &str) {
    #[cfg(target_arch = "wasm32")]
    bindings::plinth::app::dev::log(msg);
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("{msg}");
}

fn commit() {
    let ops = ui::take_ops();
    if !ops.is_empty() {
        bindings::plinth::app::ui::commit(&ops);
    }
}

fn collect_if_needed() {
    if gc::should_collect() || gc::stress_enabled() {
        gc::collect(|f| {
            reactive::trace_roots(f);
            ui::trace_roots(f);
            host::trace_roots(f);
        });
    }
}

struct Rt;

impl bindings::Guest for Rt {
    fn init(args: Vec<u8>) {
        // GC stress mode (SPEC.md §16): tests ask for a collection after
        // every `init`/`on-event` for the life of this instance.
        if plinth_protocol::init_arg::find(&args, plinth_protocol::init_arg::GC_STRESS).is_some() {
            gc::set_stress(true);
        }
        let Some(main) = MAIN.get() else { trap("the artifact has no app entry point") };
        invoke(Callable { thunk: main, env: 0 }, Val::None);
        // Hot reload (SPEC.md §13, dev builds only): restore the
        // module-level signals that `plinth dev` snapshotted from the
        // previous instance, before the first render.
        #[cfg(feature = "dev")]
        if let Some(snapshot) = plinth_protocol::init_arg::find(&args, plinth_protocol::init_arg::SNAPSHOT) {
            reactive::sig_restore(snapshot);
        }
        reactive::flush();
        commit();
        collect_if_needed();
    }

    fn on_event(ev: Vec<u8>) {
        let events = decode_events(&ev).unwrap_or_else(|_| trap("malformed event buffer"));
        for e in events {
            match e {
                plinth_protocol::Event::Ui { handler, value, .. } => {
                    ui::dispatch(handler, &value);
                    reactive::flush();
                }
                plinth_protocol::Event::Timer { timer } => {
                    host::dispatch_timer(timer);
                    reactive::flush();
                }
                #[cfg(feature = "dev")]
                plinth_protocol::Event::SnapshotRequest => {
                    ui::push_snapshot_op(reactive::sig_snapshot());
                }
                _ => {}
            }
        }
        commit();
        collect_if_needed();
    }
}

bindings::export!(Rt with_types_in bindings);

// ---------------------------------------------------------------------------
// The exported ABI. Keep it in sync with `plinth-compiler/src/rt_abi.rs`; the
// linker checks every name and type.

macro_rules! abi {
    ($( fn $name:ident ( $($arg:ident : $ty:ty),* ) $(-> $ret:ty)? $body:block )*) => {
        $(
            #[unsafe(no_mangle)]
            #[allow(clippy::missing_safety_doc)]
            pub extern "C" fn $name($($arg: $ty),*) $(-> $ret)? $body
        )*
    };
}

fn ptr(p: i32) -> u32 {
    p as u32
}

abi! {
    // -- Entry and registers --------------------------------------------------
    fn __plinth_rt_set_main(thunk: i32) { MAIN.set(Some(thunk as u32)); }
    fn __plinth_rt_arg_f64() -> f64 { ARG.get().f64() }
    fn __plinth_rt_arg_i32() -> i32 { ARG.get().i32() }
    fn __plinth_rt_ret_f64(v: f64) { RESULT.set(Val::F64(v)); }
    fn __plinth_rt_ret_i32(v: i32) { RESULT.set(Val::I32(v)); }
    fn __plinth_rt_ret_ref(p: i32) { RESULT.set(if p == 0 { Val::None } else { Val::Ref(ptr(p)) }); }
    fn __plinth_rt_log(s: i32) { log(strings::as_str(ptr(s))); }
    fn __plinth_rt_throw(s: i32) { trap(strings::as_str(ptr(s))); }

    // -- Heap --------------------------------------------------------------------
    fn __plinth_rt_alloc(type_id: i32) -> i32 { gc::alloc_user(type_id as u32) as i32 }
    fn __plinth_rt_scratch(len: i32) -> i32 { gc::buffer_alloc(len as usize) as i32 }
    fn __plinth_rt_types(table: i32, words: i32) {
        let t = unsafe { core::slice::from_raw_parts(table as *const u32, words as usize) };
        gc::register_types(t);
        gc::buffer_free(table as u32, words as usize * 4);
    }
    fn __plinth_rt_root(p: i32) { gc::add_root(ptr(p)); }
    fn __plinth_rt_closure(func: i32, env: i32) -> i32 {
        let p = gc::alloc(gc::T_CLOSURE, 16);
        unsafe {
            gc::store_u32(p + 8, func as u32);
            gc::store_u32(p + 12, env as u32);
        }
        p as i32
    }
    fn __plinth_rt_box_f64(v: f64) -> i32 {
        let p = gc::alloc(gc::T_BOX_F64, 16);
        unsafe { *((p + 8) as *mut f64) = v };
        p as i32
    }
    fn __plinth_rt_unbox_f64(p: i32) -> f64 {
        if p == 0 { trap("null value used as a number") }
        unsafe { *((p as u32 + 8) as *const f64) }
    }

    // -- Strings ---------------------------------------------------------------
    fn __plinth_rt_str_new(len: i32) -> i32 { strings::new_uninit(len as u32) as i32 }
    fn __plinth_rt_str_concat(a: i32, b: i32) -> i32 {
        let (a, b) = (strings::as_str(ptr(a)), strings::as_str(ptr(b)));
        let p = strings::new_uninit((a.len() + b.len()) as u32);
        unsafe {
            core::ptr::copy_nonoverlapping(a.as_ptr(), (p + 12) as *mut u8, a.len());
            core::ptr::copy_nonoverlapping(b.as_ptr(), (p + 12 + a.len() as u32) as *mut u8, b.len());
        }
        p as i32
    }
    fn __plinth_rt_str_eq(a: i32, b: i32) -> i32 { (a == b || strings::as_str(ptr(a)) == strings::as_str(ptr(b))) as i32 }
    fn __plinth_rt_str_cmp(a: i32, b: i32) -> i32 {
        // UTF-16 code unit order, as in JS.
        match strings::as_str(ptr(a)).encode_utf16().cmp(strings::as_str(ptr(b)).encode_utf16()) {
            core::cmp::Ordering::Less => -1,
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        }
    }
    fn __plinth_rt_str_len(a: i32) -> f64 { strings::len16(strings::as_str(ptr(a))) as f64 }
    fn __plinth_rt_str_bytes(a: i32) -> i32 { strings::as_str(ptr(a)).len() as i32 }
    fn __plinth_rt_str_from_f64(v: f64) -> i32 { strings::from_str(&strings::number_to_string(v)) as i32 }
    fn __plinth_rt_str_from_bool(v: i32) -> i32 { strings::from_str(if v != 0 { "true" } else { "false" }) as i32 }
    fn __plinth_rt_str_trim(a: i32) -> i32 { strings::from_str(strings::as_str(ptr(a)).trim()) as i32 }
    fn __plinth_rt_str_upper(a: i32) -> i32 { strings::from_str(&strings::to_upper(strings::as_str(ptr(a)))) as i32 }
    fn __plinth_rt_str_lower(a: i32) -> i32 { strings::from_str(&strings::to_lower(strings::as_str(ptr(a)))) as i32 }
    fn __plinth_rt_str_includes(a: i32, b: i32) -> i32 { strings::as_str(ptr(a)).contains(strings::as_str(ptr(b))) as i32 }
    fn __plinth_rt_str_starts(a: i32, b: i32) -> i32 { strings::as_str(ptr(a)).starts_with(strings::as_str(ptr(b))) as i32 }
    fn __plinth_rt_str_ends(a: i32, b: i32) -> i32 { strings::as_str(ptr(a)).ends_with(strings::as_str(ptr(b))) as i32 }
    fn __plinth_rt_str_slice(a: i32, start: f64, end: f64) -> i32 { strings::slice(ptr(a), start, end) as i32 }
    fn __plinth_rt_str_index_of(a: i32, b: i32) -> f64 { strings::index_of(ptr(a), ptr(b)) }
    fn __plinth_rt_str_repeat(a: i32, n: f64) -> i32 {
        let n = if n.is_finite() && n >= 0.0 { n as usize } else { trap("invalid repeat count") };
        strings::from_str(&strings::as_str(ptr(a)).repeat(n)) as i32
    }
    fn __plinth_rt_str_to_f64(a: i32) -> f64 { strings::parse_number(strings::as_str(ptr(a))) }
    fn __plinth_rt_f64_to_fixed(v: f64, digits: f64) -> i32 { strings::from_str(&strings::to_fixed(v, digits)) as i32 }

    // -- Arrays ----------------------------------------------------------------
    fn __plinth_rt_arr_new(kind: i32, cap: i32) -> i32 { arrays::new(kind as u32, cap as u32) as i32 }
    fn __plinth_rt_arr_len(a: i32) -> i32 { arrays::len(ptr(a)) as i32 }
    fn __plinth_rt_arr_get_f64(a: i32, i: i32) -> f64 { arrays::get_f64(ptr(a), i) }
    fn __plinth_rt_arr_get_i32(a: i32, i: i32) -> i32 { arrays::get_i32(ptr(a), i) }
    fn __plinth_rt_arr_set_f64(a: i32, i: i32, v: f64) { arrays::set_f64(ptr(a), i, v) }
    fn __plinth_rt_arr_set_i32(a: i32, i: i32, v: i32) { arrays::set_i32(ptr(a), i, v) }
    fn __plinth_rt_arr_push_f64(a: i32, v: f64) -> i32 { arrays::push_f64(ptr(a), v) as i32 }
    fn __plinth_rt_arr_push_i32(a: i32, v: i32) -> i32 { arrays::push_i32(ptr(a), v) as i32 }
    fn __plinth_rt_arr_pop_f64(a: i32) -> f64 { arrays::pop_f64(ptr(a)) }
    fn __plinth_rt_arr_pop_i32(a: i32) -> i32 { arrays::pop_i32(ptr(a)) }
    fn __plinth_rt_arr_slice(a: i32, start: i32, end: i32) -> i32 { arrays::slice(ptr(a), start, end) as i32 }
    fn __plinth_rt_arr_extend(dst: i32, src: i32) { arrays::extend(ptr(dst), ptr(src)) }
    fn __plinth_rt_arr_reverse(a: i32) { arrays::reverse(ptr(a)) }
    fn __plinth_rt_arr_join(a: i32, sep: i32) -> i32 {
        let a = ptr(a);
        let sep = strings::as_str(ptr(sep));
        let mut out = String::new();
        for i in 0..arrays::len(a) {
            if i > 0 { out.push_str(sep); }
            out.push_str(strings::as_str(arrays::get_i32(a, i as i32) as u32));
        }
        strings::from_str(&out) as i32
    }

    // -- Numbers ---------------------------------------------------------------
    fn __plinth_rt_f64_rem(a: f64, b: f64) -> f64 { libm::fmod(a, b) }
    fn __plinth_rt_f64_pow(a: f64, b: f64) -> f64 { libm::pow(a, b) }
    fn __plinth_rt_f64_round(v: f64) -> f64 { libm::floor(v + 0.5) }

    // -- Reactivity ------------------------------------------------------------
    fn __plinth_rt_sig_new_f64(v: f64) -> i32 { reactive::signal_new(Val::F64(v)) as i32 }
    fn __plinth_rt_sig_new_i32(v: i32) -> i32 { reactive::signal_new(Val::I32(v)) as i32 }
    fn __plinth_rt_sig_new_ref(p: i32) -> i32 { reactive::signal_new(Val::Ref(ptr(p))) as i32 }
    fn __plinth_rt_sig_get_f64(h: i32) -> f64 { reactive::signal_get(h as u32).f64() }
    fn __plinth_rt_sig_get_i32(h: i32) -> i32 { reactive::signal_get(h as u32).i32() }
    fn __plinth_rt_sig_peek_f64(h: i32) -> f64 { reactive::signal_peek(h as u32).f64() }
    fn __plinth_rt_sig_peek_i32(h: i32) -> i32 { reactive::signal_peek(h as u32).i32() }
    fn __plinth_rt_sig_set_f64(h: i32, v: f64) { reactive::signal_set(h as u32, Val::F64(v)) }
    fn __plinth_rt_sig_set_i32(h: i32, v: i32) { reactive::signal_set(h as u32, Val::I32(v)) }
    fn __plinth_rt_sig_set_ref(h: i32, p: i32) { reactive::signal_set(h as u32, Val::Ref(ptr(p))) }
    fn __plinth_rt_comp_new(thunk: i32, env: i32) -> i32 {
        reactive::computed_new(Callable { thunk: thunk as u32, env: env as u32 }) as i32
    }
    fn __plinth_rt_comp_get_f64(h: i32) -> f64 { reactive::computed_get(h as u32).f64() }
    fn __plinth_rt_comp_get_i32(h: i32) -> i32 { reactive::computed_get(h as u32).i32() }
    fn __plinth_rt_effect(thunk: i32, env: i32) {
        reactive::effect_new(reactive::EffectKind::User(Callable { thunk: thunk as u32, env: env as u32 }));
    }

    // -- UI ----------------------------------------------------------------------
    fn __plinth_rt_node(kind: i32) -> i32 { ui::create(kind as u16) as i32 }
    fn __plinth_rt_prop_str(id: i32, prop: i32, s: i32) {
        ui::set_prop(id as u32, prop as u16, plinth_protocol::Value::Str(strings::as_str(ptr(s)).to_owned()))
    }
    fn __plinth_rt_prop_f64(id: i32, prop: i32, v: f64) { ui::set_prop(id as u32, prop as u16, plinth_protocol::Value::Number(v)) }
    fn __plinth_rt_prop_int(id: i32, prop: i32, v: i32) { ui::set_prop(id as u32, prop as u16, plinth_protocol::Value::Int(v)) }
    fn __plinth_rt_prop_bool(id: i32, prop: i32, v: i32) { ui::set_prop(id as u32, prop as u16, plinth_protocol::Value::Bool(v != 0)) }
    fn __plinth_rt_prop_enum(id: i32, prop: i32, v: i32) { ui::set_prop(id as u32, prop as u16, plinth_protocol::Value::Enum(v as u16)) }
    fn __plinth_rt_text(id: i32, s: i32) { ui::set_text(id as u32, strings::as_str(ptr(s))) }
    fn __plinth_rt_append(parent: i32, child: i32) { ui::append(parent as u32, child as u32) }
    fn __plinth_rt_region(parent: i32, thunk: i32, env: i32) {
        ui::region(parent as u32, Callable { thunk: thunk as u32, env: env as u32 })
    }
    fn __plinth_rt_listen(id: i32, ev: i32, thunk: i32, env: i32) {
        ui::listen(id as u32, ev as u16, Callable { thunk: thunk as u32, env: env as u32 })
    }
    fn __plinth_rt_bind(id: i32, sig: i32, kind: i32) {
        let kind = match kind {
            1 => ui::BindKind::Bool,
            2 => ui::BindKind::Num,
            _ => ui::BindKind::Str,
        };
        ui::bind(id as u32, sig as u32, kind)
    }
    fn __plinth_rt_list(id: i32, items_t: i32, items_e: i32, key_t: i32, key_e: i32, row_t: i32, row_e: i32, empty_t: i32, empty_e: i32) {
        let c = |t: i32, e: i32| Callable { thunk: t as u32, env: e as u32 };
        let empty = (empty_t != 0).then(|| c(empty_t, empty_e));
        ui::list(id as u32, c(items_t, items_e), c(key_t, key_e), c(row_t, row_e), empty)
    }
    fn __plinth_rt_set_root(screen: i32, id: i32) { ui::set_root(screen as u32, id as u32) }
    fn __plinth_rt_navigate(screen: i32) { ui::navigate(screen as u32) }

    // -- UI API 1.2: structure and stack navigation ----------------------------
    fn __plinth_rt_mark_primary(screen: i32) { ui::mark_primary(screen as u32) }
    fn __plinth_rt_navigate_push(screen: i32) { ui::navigate_push(screen as u32) }
    fn __plinth_rt_navigate_back() { ui::navigate_back() }
    // -- Host APIs (plinth:time, plinth:store, plinth:clipboard; SPEC.md §8.5) --
    fn __plinth_rt_time_now() -> f64 { host::now() }
    fn __plinth_rt_time_monotonic_now() -> f64 { host::monotonic_now() }
    fn __plinth_rt_set_timer(thunk: i32, env: i32, ms: f64, repeat: i32) -> f64 {
        host::set_timer(Callable { thunk: thunk as u32, env: env as u32 }, ms, repeat != 0)
    }
    fn __plinth_rt_clear_timer(id: f64) { host::clear_timer(id) }
    fn __plinth_rt_kv_get(key: i32) -> i32 { host::kv_get(key) }
    fn __plinth_rt_kv_set(key: i32, value: i32) { host::kv_set(key, value) }
    fn __plinth_rt_kv_delete(key: i32) { host::kv_delete(key) }
    fn __plinth_rt_kv_keys() -> i32 { host::kv_keys() }
    fn __plinth_rt_clipboard_write_text(text: i32) { host::clipboard_write_text(text) }
    fn __plinth_rt_clipboard_read_text() -> i32 { host::clipboard_read_text() }
    fn __plinth_rt_kv_last_error() -> i32 { host::kv_last_error() }
    fn __plinth_rt_clipboard_last_error() -> i32 { host::clipboard_last_error() }

    // -- JSON (plinth:core, SPEC.md §4.7) --------------------------------------
    fn __plinth_rt_json_num_str(v: f64) -> i32 { strings::from_str(&strings::json_number_to_string(v)) as i32 }
    fn __plinth_rt_json_quote_str(a: i32) -> i32 { strings::from_str(&strings::json_quote(strings::as_str(ptr(a)))) as i32 }
}

// Hot reload (SPEC.md §13): a dev-only ABI function, not declared with the
// `abi!` macro above because it must not exist at all in a release build
// (the macro cannot carry a `#[cfg(...)]` into its repetition). `lower.rs`
// emits a call to it only when compiling in dev mode, and only a dev build
// of `plinth-rt` (the `dev` feature) exports it, so a release app artifact
// never references, and never contains, this function.
#[cfg(feature = "dev")]
#[unsafe(no_mangle)]
pub extern "C" fn __plinth_rt_sig_register(id: i32, key: i32, shape: i32) {
    reactive::sig_register(id as u32, key, shape);
}
