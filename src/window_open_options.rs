use crate::Size;

/// The options for opening a new window
#[derive(Debug, Clone)]
pub struct WindowOpenOptions {
    pub title: String,

    /// The physical size of the window.
    pub size: Size,

    /// If provided, then an OpenGL context will be created for this window. You'll be able to
    /// access this context through [crate::Window::gl_context].
    #[cfg(feature = "opengl")]
    pub gl_config: Option<crate::gl::GlConfig>,
}
