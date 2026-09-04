pub mod util;
pub mod gui;
pub mod body;
pub mod interop;
pub mod sim;
pub mod presentation;
pub mod camera;
pub mod catalog;
/// Re-exported so existing `crate::foundations::…` paths keep resolving.
/// New code should prefer `em_foundations::…` directly.
pub use em_foundations as foundations;
