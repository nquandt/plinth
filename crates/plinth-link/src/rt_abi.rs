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
    // -- plinth:dialog (SPEC.md §8.4, §8.5) ---------------------------------
    ("dialog_alert", &[I32, I32, I32], &[]),
    ("dialog_confirm", &[I32, I32, I32], &[]),
    ("dialog_prompt", &[I32, I32, I32], &[]),
    // -- more string methods (gap #4) ---------------------------------------
    ("str_last_index_of", &[I32, I32], &[F64]),
    ("str_replace", &[I32, I32, I32], &[I32]),
    ("str_replace_all", &[I32, I32, I32], &[I32]),
    ("str_pad_start", &[I32, F64, I32], &[I32]),
    ("str_pad_end", &[I32, F64, I32], &[I32]),
    ("str_split", &[I32, I32], &[I32]),
    // -- plinth:net (SPEC.md §8.4, §8.5, §11) -------------------------------
    ("net_fetch", &[I32, I32, I32, I32, I32, I32], &[]),
    ("net_result_ok", &[], &[I32]),
    ("net_result_status", &[], &[F64]),
    ("net_result_text", &[], &[I32]),
    ("net_result_error", &[], &[I32]),
    // -- plinth:time date/time additions (gap #5) ---------------------------
    ("tz_offset_minutes", &[F64], &[F64]),
    ("date_field", &[F64, I32, I32], &[F64]),
    ("make_date", &[F64, F64, F64, F64, F64, F64], &[F64]),
    ("format_date", &[F64, I32, I32], &[I32]),
    ("parse_date", &[I32], &[I32]),
    // -- plinth:hub (`docs/HUB.md` §4.1, §12.2; capability: hub.manage) -----
    ("hub_list_apps", &[], &[I32]),
    ("hub_launch", &[I32], &[]),
    ("hub_set_grant", &[I32, I32, I32], &[]),
    ("hub_block", &[I32], &[]),
    ("hub_unblock", &[I32], &[]),
    ("hub_last_error", &[], &[I32]),
    // -- plinth:hub, core 1.8 (`docs/HUB.md` §9.1, §5.2, H3 step 2) ---------
    ("hub_list_groups", &[], &[I32]),
    ("hub_create_group", &[I32], &[]),
    ("hub_set_group", &[I32, I32, I32], &[]),
    ("hub_remove", &[I32], &[]),
    ("hub_search", &[I32, I32, I32], &[]),
    ("hub_install", &[I32, I32, I32], &[]),
    // -- plinth:hub, core 1.9 (`docs/HUB.md` §4.1, §7.4, §9.2) --------------
    ("hub_app_info", &[I32], &[I32]),
    ("hub_pin", &[I32, I32], &[]),
    ("hub_block_publisher", &[I32], &[]),
    ("hub_unblock_publisher", &[I32], &[]),
    ("hub_check_updates", &[I32, I32, I32], &[]),
    ("hub_update", &[I32, I32, I32], &[]),
    // -- Errors (SPEC.md §5.6, core 1.10) ------------------------------------
    ("uncaught", &[I32, I32], &[]),
    // -- async/await (SPEC.md §4.5, core 1.10) -------------------------------
    ("set_drain", &[I32, I32], &[]),
    ("report", &[I32], &[]),
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
    ("dialog_alert", 3),
    ("dialog_confirm", 3),
    ("dialog_prompt", 3),
    ("str_last_index_of", 4),
    ("str_replace", 4),
    ("str_replace_all", 4),
    ("str_pad_start", 4),
    ("str_pad_end", 4),
    ("str_split", 4),
    ("net_fetch", 5),
    ("net_result_ok", 5),
    ("net_result_status", 5),
    ("net_result_text", 5),
    ("net_result_error", 5),
    ("tz_offset_minutes", 6),
    ("date_field", 6),
    ("make_date", 6),
    ("format_date", 6),
    ("parse_date", 6),
    ("hub_list_apps", 7),
    ("hub_launch", 7),
    ("hub_set_grant", 7),
    ("hub_block", 7),
    ("hub_unblock", 7),
    ("hub_last_error", 7),
    ("hub_list_groups", 8),
    ("hub_create_group", 8),
    ("hub_set_group", 8),
    ("hub_remove", 8),
    ("hub_search", 8),
    ("hub_install", 8),
    ("hub_app_info", 9),
    ("hub_pin", 9),
    ("hub_block_publisher", 9),
    ("hub_unblock_publisher", 9),
    ("hub_check_updates", 9),
    ("hub_update", 9),
    ("uncaught", 10),
    ("set_drain", 10),
    ("report", 10),
];

/// The minor version that added `name` (0 for the functions of 1.0).
pub fn added_in(name: &str) -> u32 {
    ADDED_IN.iter().find(|(n, _)| *n == name).map_or(0, |(_, m)| *m)
}
pub const CORE_MINOR: u32 = 10;

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
