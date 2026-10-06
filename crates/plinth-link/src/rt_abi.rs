//! The app ABI: the `plinth-rt` functions and constants that generated code
//! uses (SPEC.md §10.4). Keep this table in sync with `plinth-rt/src/lib.rs`.
//! The linker checks each name and type against the runtime module, so a
//! mismatch fails at link time.
//!
//! **Compatibility rule.** A `.plnt` is built one time and runs on every host
//! that has a core (a `plinth-rt` build) of its major version (SPEC.md
//! §10.5). Thus, inside one major version, only ADD functions (at the end of
//! `FUNCTIONS`) and constants, and increment `CORE_MINOR` (and the version in
//! `plinth-rt/src/lib.rs`) for each addition. Do not remove a function,
//! change its type, or change what it does, and do not change a constant or
//! the object layout (`HEADER`, `STR_BYTES`, the type ids). A change of that
//! kind needs a new major version; hosts then install both cores.

use wasm_encoder::ValType::{self, F64, I32};

/// `(name without the "__plinth_rt_" prefix, params, results)`.
pub const FUNCTIONS: &[(&str, &[ValType], &[ValType])] = &[
    ("set_main", &[I32], &[]),
    ("arg_f64", &[], &[F64]),
    ("arg_i32", &[], &[I32]),
    ("ret_f64", &[F64], &[]),
    ("ret_i32", &[I32], &[]),
    ("ret_ref", &[I32], &[]),
    ("log", &[I32], &[]),
    ("throw", &[I32], &[]),
    ("alloc", &[I32], &[I32]),
    ("scratch", &[I32], &[I32]),
    ("types", &[I32, I32], &[]),
    ("root", &[I32], &[]),
    ("closure", &[I32, I32], &[I32]),
    ("box_f64", &[F64], &[I32]),
    ("unbox_f64", &[I32], &[F64]),
    ("str_new", &[I32], &[I32]),
    ("str_concat", &[I32, I32], &[I32]),
    ("str_eq", &[I32, I32], &[I32]),
    ("str_cmp", &[I32, I32], &[I32]),
    ("str_len", &[I32], &[F64]),
    ("str_bytes", &[I32], &[I32]),
    ("str_from_f64", &[F64], &[I32]),
    ("str_from_bool", &[I32], &[I32]),
    ("str_trim", &[I32], &[I32]),
    ("str_upper", &[I32], &[I32]),
    ("str_lower", &[I32], &[I32]),
    ("str_includes", &[I32, I32], &[I32]),
    ("str_starts", &[I32, I32], &[I32]),
    ("str_ends", &[I32, I32], &[I32]),
    ("str_slice", &[I32, F64, F64], &[I32]),
    ("str_index_of", &[I32, I32], &[F64]),
    ("str_repeat", &[I32, F64], &[I32]),
    ("str_to_f64", &[I32], &[F64]),
    ("f64_to_fixed", &[F64, F64], &[I32]),
    ("arr_new", &[I32, I32], &[I32]),
    ("arr_len", &[I32], &[I32]),
    ("arr_get_f64", &[I32, I32], &[F64]),
    ("arr_get_i32", &[I32, I32], &[I32]),
    ("arr_set_f64", &[I32, I32, F64], &[]),
    ("arr_set_i32", &[I32, I32, I32], &[]),
    ("arr_push_f64", &[I32, F64], &[I32]),
    ("arr_push_i32", &[I32, I32], &[I32]),
    ("arr_pop_f64", &[I32], &[F64]),
    ("arr_pop_i32", &[I32], &[I32]),
    ("arr_slice", &[I32, I32, I32], &[I32]),
    ("arr_extend", &[I32, I32], &[]),
    ("arr_reverse", &[I32], &[]),
    ("arr_join", &[I32, I32], &[I32]),
    ("f64_rem", &[F64, F64], &[F64]),
    ("f64_pow", &[F64, F64], &[F64]),
    ("f64_round", &[F64], &[F64]),
    ("sig_new_f64", &[F64], &[I32]),
    ("sig_new_i32", &[I32], &[I32]),
    ("sig_new_ref", &[I32], &[I32]),
    ("sig_get_f64", &[I32], &[F64]),
    ("sig_get_i32", &[I32], &[I32]),
    ("sig_peek_f64", &[I32], &[F64]),
    ("sig_peek_i32", &[I32], &[I32]),
    ("sig_set_f64", &[I32, F64], &[]),
    ("sig_set_i32", &[I32, I32], &[]),
    ("sig_set_ref", &[I32, I32], &[]),
    ("comp_new", &[I32, I32], &[I32]),
    ("comp_get_f64", &[I32], &[F64]),
    ("comp_get_i32", &[I32], &[I32]),
    ("effect", &[I32, I32], &[]),
    ("node", &[I32], &[I32]),
    ("prop_str", &[I32, I32, I32], &[]),
    ("prop_f64", &[I32, I32, F64], &[]),
    ("prop_int", &[I32, I32, I32], &[]),
    ("prop_bool", &[I32, I32, I32], &[]),
    ("prop_enum", &[I32, I32, I32], &[]),
    ("text", &[I32, I32], &[]),
    ("append", &[I32, I32], &[]),
    ("region", &[I32, I32, I32], &[]),
    ("listen", &[I32, I32, I32, I32], &[]),
    ("bind", &[I32, I32, I32], &[]),
    ("list", &[I32, I32, I32, I32, I32, I32, I32, I32, I32], &[]),
    ("set_root", &[I32, I32], &[]),
    ("navigate", &[I32], &[]),
    // -- UI API 1.2: structure and stack navigation ------------------------
    ("mark_primary", &[I32], &[]),
    ("navigate_push", &[I32], &[]),
    ("navigate_back", &[], &[]),
    ("time_now", &[], &[F64]),
    ("time_monotonic_now", &[], &[F64]),
    ("set_timer", &[I32, I32, F64, I32], &[F64]),
    ("clear_timer", &[F64], &[]),
    ("kv_get", &[I32], &[I32]),
    ("kv_set", &[I32, I32], &[]),
    ("kv_delete", &[I32], &[]),
    ("kv_keys", &[], &[I32]),
    ("clipboard_write_text", &[I32], &[]),
    ("clipboard_read_text", &[], &[I32]),
    ("kv_last_error", &[], &[I32]),
    ("clipboard_last_error", &[], &[I32]),
    // -- JSON (SPEC.md §4.7) -----------------------------------------------
    ("json_num_str", &[F64], &[I32]),
    ("json_quote_str", &[I32], &[I32]),
    // -- JSON.parse (SPEC.md §4.7) -----------------------------------------
    ("json_begin", &[I32], &[]),
    ("json_ok", &[], &[I32]),
    ("json_finish", &[], &[I32]),
    ("json_fail", &[], &[]),
    ("json_peek_kind", &[], &[I32]),
    ("json_read_null", &[], &[]),
    ("json_read_bool", &[], &[I32]),
    ("json_read_num", &[], &[F64]),
    ("json_read_int", &[], &[I32]),
    ("json_read_str", &[], &[I32]),
    ("json_skip_value", &[], &[]),
    ("json_arr_begin", &[], &[]),
    ("json_arr_next", &[], &[I32]),
    ("json_obj_begin", &[], &[]),
    ("json_obj_next_key", &[], &[I32]),
];

/// Hot reload (SPEC.md §13): functions that only a dev build of
/// `plinth-rt` exports (the `dev` cargo feature). `lower.rs` emits calls
/// to them only when compiling with `dev: true`, and `link::layout` only
/// requires them from the dev runtime blob, so a release app artifact
/// never references, and never contains, this code.
pub const DEV_FUNCTIONS: &[(&str, &[ValType], &[ValType])] = &[("sig_register", &[I32, I32, I32], &[])];

pub const PREFIX: &str = "__plinth_rt_";

/// The core version that this compiler targets. An app module declares it
/// in its `plinth-core` custom section (SPEC.md §10.4, §10.5).
pub const CORE_MAJOR: u32 = 1;

/// The minor version in which each function after 1.0 was added. An app
/// needs the highest minor version among the functions that it imports, so
/// an app that uses only 1.0 functions still runs on a 1.0 core. Add each
/// new function here with the new `CORE_MINOR`.
pub const ADDED_IN: &[(&str, u32)] = &[
    ("json_num_str", 1),
    ("json_quote_str", 1),
    ("json_begin", 2),
    ("json_ok", 2),
    ("json_finish", 2),
    ("json_fail", 2),
    ("json_peek_kind", 2),
    ("json_read_null", 2),
    ("json_read_bool", 2),
    ("json_read_num", 2),
    ("json_read_int", 2),
    ("json_read_str", 2),
    ("json_skip_value", 2),
    ("json_arr_begin", 2),
    ("json_arr_next", 2),
    ("json_obj_begin", 2),
    ("json_obj_next_key", 2),
];

/// The minor version that added `name` (0 for the functions of 1.0).
pub fn added_in(name: &str) -> u32 {
    ADDED_IN.iter().find(|(n, _)| *n == name).map_or(0, |(_, m)| *m)
}
pub const CORE_MINOR: u32 = 2;

/// Array kinds for `arr_new` (the runtime's built-in type ids).
pub const ARR_F64: i32 = 1;
pub const ARR_I32: i32 = 2;
pub const ARR_REF: i32 = 3;
/// The runtime's built-in type id for a string object (mirrors
/// `plinth-rt/src/gc.rs`'s `T_STRING`), used to narrow a union member.
pub const T_STRING: u32 = 0;

/// The first type id for compiler-defined types.
pub const FIRST_USER_TYPE: u32 = 16;
/// The size of the object header.
pub const HEADER: u32 = 8;
/// The offset of the bytes in a string object.
pub const STR_BYTES: u32 = 12;
