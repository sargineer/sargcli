//! Hooks turn "ask sarg first" from advice into mechanism. The logic is
//! agent-neutral (`core`), and each agent's hook protocol gets a thin
//! adapter (`claude` so far). Nothing here guesses which agent is calling:
//! the adapter is chosen explicitly by `sarg hook <agent> <event>`.

pub mod claude;
pub mod core;
