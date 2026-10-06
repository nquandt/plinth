//! Runtime cores and the app linker (SPEC.md §10.4, §10.5).
//!
//! Every host needs this crate to run a `.plnt`: it embeds the built-in core
//! (`plinth-rt`), finds the installed cores, checks an app module, and links
//! it into its core. It has no compiler in it, so a runner stays small.

pub mod capabilities;
pub mod cores;
pub mod link;
pub mod rt_abi;
pub mod split;
