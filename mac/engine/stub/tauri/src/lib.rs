//! The few `tauri` paths the shared engine sources name. Nothing here touches a webview.
pub use tauri_macros_stub::command;

pub mod async_runtime {
    use std::convert::Infallible;
    use std::future::{ready, Ready};

    /// Runs the work at once: the helper handles one unit and has nothing else to keep responsive.
    pub fn spawn_blocking<F: FnOnce() -> R, R>(f: F) -> Ready<Result<R, Infallible>> {
        ready(Ok(f()))
    }

    /// Only ever given futures that finish on their first poll.
    pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
        let mut f = std::pin::pin!(f);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::yield_now();
        }
    }
}
