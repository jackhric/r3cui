//! Kodi's methods the client uses, one file per namespace, each a struct of its parameters implementing `Method`.
//! Names and parameters follow the vendored schema; `tests/schema.rs` checks every one against it.

pub mod gui;
pub mod input;
pub mod library;
pub mod player;
pub mod system;

/// A method that takes no parameters and answers "OK" (or anything we don't read).
macro_rules! simple {
    ($(#[$doc:meta])* $name:ident = $method:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, Default, serde::Serialize)]
        pub struct $name {}
        impl crate::rpc::Method for $name {
            const NAME: &'static str = $method;
            type Output = serde::de::IgnoredAny;
        }
    };
}
pub(crate) use simple;
