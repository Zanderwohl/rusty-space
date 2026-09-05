//! The two windows that show and change one body: the Viewer and the Editor.
//!
//! They share a body chooser and a formatting vocabulary, so a value reads the same
//! whichever window it is in, and the selection follows you between them.

pub mod editor;
pub mod fields;
pub mod picker;
pub mod viewer;

pub use editor::editor_window;
pub use picker::BodyPickerState;
pub use viewer::viewer_window;
