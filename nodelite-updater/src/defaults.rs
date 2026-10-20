//! Conservative defaults keep enrollment explicit and bound health trials.

pub(crate) fn disabled() -> bool {
    false
}
pub(crate) fn stable() -> String {
    "stable".into()
}
pub(crate) fn timeout() -> u64 {
    60
}
pub(crate) fn stability() -> u64 {
    10
}
