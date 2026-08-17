//! Agents' computers, behind a provider boundary.
//!
//! The runtime asks for "this agent's machine" and gets a `Machine` it can run
//! commands on. Who actually runs that machine — E2B today, a local container
//! runtime later — is a `ComputerProvider`, and nothing above this module
//! knows which one it got.

pub mod provider;

pub use provider::Output;
