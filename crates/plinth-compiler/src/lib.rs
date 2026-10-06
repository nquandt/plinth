//! The Plinth TS compiler (SPEC.md §5).

pub mod ast;
pub mod check;
pub mod controls;
pub mod diag;
pub mod driver;
pub mod link;
pub mod parse;
pub mod rt_abi;
pub mod tir;
pub mod types;
