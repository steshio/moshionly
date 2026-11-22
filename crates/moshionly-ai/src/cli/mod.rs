//! Subscription providers: agent CLIs that the user installed and signed in
//! to. Moshionly runs the binary and reads its output, nothing more.

// The providers that use these helpers arrive in the following commits.
#![allow(dead_code)]

pub(crate) mod discover;
pub(crate) mod process;
