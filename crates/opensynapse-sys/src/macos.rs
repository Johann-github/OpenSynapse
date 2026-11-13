//! macOS side: effective user id (ADR-0012).

#![allow(unsafe_code)] // One libSystem call; see the invariant below.

unsafe extern "C" {
    /// `uid_t geteuid(void)` from libSystem (always linked); `uid_t` is a
    /// 32-bit unsigned integer on Darwin.
    safe fn geteuid() -> u32;
}

/// `true` when the effective user is root.
pub(crate) fn is_root() -> bool {
    // geteuid takes no arguments, cannot fail and touches no memory of ours,
    // so it is declared `safe` above.
    geteuid() == 0
}
