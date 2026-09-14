//! Generate defaults once so an omitted section and an empty section behave identically.

macro_rules! default_fns {
    ($($(#[$attr:meta])* $name:ident -> $ty:ty = $value:expr;)+) => {
        $($(#[$attr])* pub(super) fn $name() -> $ty { $value })+
    };
}

#[cfg(feature = "server-config")]
macro_rules! config_section {
    ($vis:vis struct $name:ident { $($field:ident: $ty:ty = $value:expr,)* }) => {
        #[derive(Debug, Clone, serde::Deserialize)]
        #[serde(default, deny_unknown_fields)]
        $vis struct $name { $($field: $ty,)* }

        impl Default for $name {
            fn default() -> Self { Self { $($field: $value,)* } }
        }
    };
}
