//! The `plinth-rt` functions that generated code calls. Keep this table in
//! sync with `plinth-rt/src/lib.rs`. The linker checks each name and type
//! against the runtime module, so a mismatch fails at link time.

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
];

pub const PREFIX: &str = "__plinth_rt_";

/// Array kinds for `arr_new` (the runtime's built-in type ids).
pub const ARR_F64: i32 = 1;
pub const ARR_I32: i32 = 2;
pub const ARR_REF: i32 = 3;

/// The first type id for compiler-defined types.
pub const FIRST_USER_TYPE: u32 = 16;
/// The size of the object header.
pub const HEADER: u32 = 8;
/// The offset of the bytes in a string object.
pub const STR_BYTES: u32 = 12;
