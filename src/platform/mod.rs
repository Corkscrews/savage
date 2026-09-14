#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{bind_proxy, install, on_ready};

#[cfg(not(target_os = "macos"))]
mod stub {
    use winit::event_loop::EventLoopProxy;

    use crate::UserEvent;

    pub fn install() {}

    pub fn bind_proxy(_proxy: EventLoopProxy<UserEvent>) {}

    pub fn on_ready() {}
}

#[cfg(not(target_os = "macos"))]
pub use stub::{bind_proxy, install, on_ready};
