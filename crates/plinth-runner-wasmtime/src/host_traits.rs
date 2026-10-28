//! The host-side traits of the `plinth:app` world, written by hand, for a
//! build without wasmtime (`wasmtime` feature off, PRODUCTION R5).
//!
//! With the `wasmtime` feature, `wasmtime::component::bindgen!` makes these
//! traits from `wit/plinth/app.wit`. A build for iOS has no wasmtime (no
//! JIT), so the same paths, names and signatures are written here. The impls
//! on `HostState` in `lib.rs` and the shims in `wasmi_engine.rs` then compile
//! in both builds without a change. When `app.wit` changes, change this file
//! too.
//!
//! Two checks find a mismatch:
//! - `cargo build -p plinth-runner-wasmtime --no-default-features --features
//!   wasmi` shows it as a compile error.
//! - The test `tests/host_traits_drift.rs` runs in the default build. It reads
//!   `wit/plinth/app.wit` and this file as text and compares the interfaces,
//!   the function names, the parameter names and types, the return types, and
//!   the cases of each `enum` and `variant`. The order of the items does not
//!   matter. Write each `fn` on one line, and write each type in the same
//!   form as `bindgen!` does (`Result<(), HostError>`, `Vec<(String, String)>`).
//!   If the WIT uses a new type, teach `rust_type` in the test about it.

pub mod plinth {
    pub mod app {
        pub mod error {
            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub enum DeniedReason {
                Undeclared,
                Refused,
                Unsupported,
            }

            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub enum HostError {
                Denied(DeniedReason),
            }

            pub trait Host {
                fn report(&mut self, message: String);
            }
        }

        pub mod ui {
            pub trait Host {
                fn commit(&mut self, ops: Vec<u8>);
            }
        }

        pub mod dev {
            pub trait Host {
                fn log(&mut self, msg: String);
            }
        }

        pub mod time {
            use super::error::HostError;

            pub trait Host {
                fn now(&mut self) -> u64;
                fn monotonic_now(&mut self) -> u64;
                fn set_timer(&mut self, ms: u32, repeat: bool) -> Result<u32, HostError>;
                fn cancel_timer(&mut self, timer: u32);
                fn set_frame_timer(&mut self) -> Result<u32, HostError>;
                fn timezone_offset(&mut self, ms: i64) -> i32;
            }
        }

        pub mod store {
            use super::error::HostError;

            pub trait Host {
                fn kv_get(&mut self, key: String) -> Result<Option<String>, HostError>;
                fn kv_set(&mut self, key: String, value: String) -> Result<(), HostError>;
                fn kv_delete(&mut self, key: String) -> Result<(), HostError>;
                fn kv_keys(&mut self) -> Result<Vec<String>, HostError>;
            }
        }

        pub mod clipboard {
            use super::error::HostError;

            pub trait Host {
                fn write_text(&mut self, text: String) -> Result<(), HostError>;
                fn read_text(&mut self) -> Result<Option<String>, HostError>;
            }
        }

        pub mod hub {
            use super::error::HostError;

            pub trait Host {
                fn list_apps(&mut self) -> Result<String, HostError>;
                fn launch(&mut self, id: String) -> Result<(), HostError>;
                fn set_grant(&mut self, id: String, capability: String, allowed: bool) -> Result<(), HostError>;
                fn block(&mut self, id: String) -> Result<(), HostError>;
                fn unblock(&mut self, id: String) -> Result<(), HostError>;
                fn list_groups(&mut self) -> Result<String, HostError>;
                fn create_group(&mut self, name: String) -> Result<(), HostError>;
                fn set_group(&mut self, id: String, group: String, member: bool) -> Result<(), HostError>;
                fn remove(&mut self, id: String) -> Result<(), HostError>;
                fn search(&mut self, query: String) -> u32;
                fn install(&mut self, id: String) -> u32;
                fn app_info(&mut self, id: String) -> Result<String, HostError>;
                fn pin(&mut self, id: String, version: String) -> Result<(), HostError>;
                fn block_publisher(&mut self, key: String) -> Result<(), HostError>;
                fn unblock_publisher(&mut self, key: String) -> Result<(), HostError>;
                fn check_updates(&mut self, id: String) -> u32;
                fn update(&mut self, id: String) -> u32;
            }
        }

        pub mod dialog {
            pub trait Host {
                fn alert(&mut self, message: String) -> u32;
                fn confirm(&mut self, message: String) -> u32;
                fn prompt(&mut self, message: String) -> u32;
            }
        }

        pub mod notify {
            use super::error::HostError;

            pub trait Host {
                fn show(&mut self, title: String, body: Option<String>, tag: Option<String>) -> u32;
                fn cancel(&mut self, tag: String) -> Result<(), HostError>;
            }
        }

        pub mod secrets {
            pub trait Host {
                fn set(&mut self, name: String, value: String) -> u32;
                fn get(&mut self, name: String) -> u32;
                fn remove(&mut self, name: String) -> u32;
                fn names(&mut self) -> u32;
            }
        }

        pub mod net {
            pub trait Host {
                fn fetch(&mut self, url: String, method: String, headers: Vec<(String, String)>, body: Option<String>) -> u32;
            }
        }

        pub mod locale {
            pub trait Host {
                fn tag(&mut self) -> String;
                fn is_rtl(&mut self) -> bool;
                fn format_number(&mut self, value: f64, options: String) -> String;
                fn format_date(&mut self, ms: f64, options: String) -> String;
                fn plural_category(&mut self, value: f64, kind: String) -> String;
                fn format_list(&mut self, items: String, kind: String) -> String;
                fn compare(&mut self, a: String, b: String) -> i32;
            }
        }

        pub mod app_info {
            pub trait Host {
                fn data_version(&mut self) -> Option<String>;
                fn mark_data_version(&mut self);
            }
        }

        pub mod sql {
            pub trait Host {
                fn run(&mut self, kind: u8, statement: String, params: String) -> u32;
            }
        }

        pub mod files {
            pub trait Host {
                fn read(&mut self, path: String) -> u32;
                fn write(&mut self, path: String, text: String) -> u32;
                fn list_dir(&mut self, dir: String) -> u32;
                fn stat(&mut self, path: String) -> u32;
                fn remove(&mut self, path: String) -> u32;
            }
        }
    }
}
