//! Architecture quarantine: everything CPU-specific lives under here
//! (CODING-CONVENTIONS, RISK R10). Today: x86_64. A future arch gets a
//! sibling module behind the same interfaces.

pub mod x86_64;
