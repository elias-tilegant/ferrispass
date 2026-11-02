//! Audited platform modules. Each file here wraps one system API that has no
//! safe binding in our dependencies, opts in to `unsafe` once with
//! `#![allow(unsafe_code)]`, and says why in its header. `tests/architecture.rs`
//! keeps the list, and SECURITY.md describes every entry under "Unsafe code".
//! New `unsafe` code goes here and nowhere else.

pub mod secure_input;
