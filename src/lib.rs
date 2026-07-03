//! straitjacket — flags weird code that LLMs tend to generate, ahead of time.
//!
//! The model: a small set of hardcoded [`rules`], each a deterministic pattern over
//! source lines, run by an [`Engine`] that uses a single `RegexSet` as a per-line
//! prefilter so every regex-backed rule is tested in one pass. Rules are generic —
//! no framework or single-language assumptions — so the same binary drops into any
//! repo's CI. Each rule honours a same-line `straitjacket-allow` escape hatch.

pub mod config;
pub mod duplication;
pub mod engine;
pub mod finding;
pub mod prop_graph;
pub mod react;
pub mod rules;
pub mod sarif;
pub mod slop_prose;
pub mod walk;

pub use config::Config;
pub use engine::Engine;
pub use finding::{Finding, Severity};
