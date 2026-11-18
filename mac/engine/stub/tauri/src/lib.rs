//! The few `tauri` paths the shared engine sources name. Nothing here touches a webview.
pub use tauri_macros_stub::command;

pub mod async_runtime {
    use std::future::{ready, Ready};

    /// Same shape as Tauri's join error. Never produced here, but an inhabited type keeps the
    /// shared code's `.map_err(..)` reachable (with `Infallible`, newer Rust rejects it).
    #[derive(Debug)]
    pub struct JoinError(());

    impl std::fmt::Display for JoinError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("task failed")
        }
    }

    impl std::error::Error for JoinError {}

    /// Runs the work at once: the helper handles one unit and has nothing else to keep responsive.
    pub fn spawn_blocking<F: FnOnce() -> R, R>(f: F) -> Ready<Result<R, JoinError>> {
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
