//! Shared macros for the non-native stub implementations.
//!
//! Every built-in source is reachable on every target so a portable chain
//! can be written without `cfg` gates at the call site. On platforms
//! where the source has nothing to contribute, the "stub" version
//! compiles in and its `probe()` returns `Ok(None)`. Both macros below
//! generate that boilerplate.

/// Zero-sized stub source. Emits a unit struct with `new` / `Default` /
/// a `Source` impl whose `probe` always returns `Ok(None)`.
macro_rules! unit_stub {
    ($(#[$meta:meta])* $name:ident, $kind:expr) => {
        $(#[$meta])*
        #[derive(Debug, Default, Clone)]
        pub struct $name {
            _priv: (),
        }

        impl $name {
            /// Construct the source.
            #[must_use]
            pub fn new() -> Self {
                Self { _priv: () }
            }
        }

        impl $crate::source::Source for $name {
            fn kind(&self) -> $crate::source::SourceKind {
                $kind
            }
            fn probe(&self) -> Result<Option<$crate::source::Probe>, $crate::error::Error> {
                Ok(None)
            }
        }
    };
}

/// Path-configurable stub source. Mirrors the `file_source!` shape in
/// `linux.rs` but with a no-op `probe()`.
macro_rules! path_stub {
    ($(#[$meta:meta])* $name:ident, $kind:expr, $default:expr) => {
        $(#[$meta])*
        #[derive(Debug, Clone)]
        pub struct $name {
            path: std::path::PathBuf,
        }

        impl $name {
            #[doc = concat!("Construct with the standard default path (`", $default, "`).")]
            #[must_use]
            pub fn new() -> Self {
                Self { path: std::path::PathBuf::from($default) }
            }

            /// Construct at a caller-supplied path.
            #[must_use]
            pub fn at(path: impl Into<std::path::PathBuf>) -> Self {
                Self { path: path.into() }
            }

            /// The configured path.
            #[must_use]
            pub fn path(&self) -> &std::path::Path {
                &self.path
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl $crate::source::Source for $name {
            fn kind(&self) -> $crate::source::SourceKind {
                $kind
            }
            fn probe(&self) -> Result<Option<$crate::source::Probe>, $crate::error::Error> {
                Ok(None)
            }
        }
    };
}

pub(crate) use path_stub;
pub(crate) use unit_stub;
