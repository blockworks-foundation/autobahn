use std::future::Future;
use std::process;
use tokio::task::JoinHandle;

use crate::prelude::*;

#[inline(always)]
#[allow(unused_variables)]
pub fn tokio_spawn<T>(name: &str, future: T) -> JoinHandle<T::Output>
where
    T: Future + Send + 'static,
    T::Output: Send + 'static,
{
    #[cfg(not(tokio_unstable))]
    {
        tokio::spawn(future)
    }

    #[cfg(tokio_unstable)]
    {
        tokio::task::Builder::new()
            .name(name)
            .spawn(future)
            .expect("always Ok")
    }
}

/// Panics if the local time is < unix epoch start
pub fn millis_since_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

pub fn print_git_version() {
    match option_env!("GITHUB_SHA") {
        Some(sha) => {
            info!("version is {}[github]", sha,);
        }
        None => {
            info!(
                "version is {}[{}{}]",
                env!("VERGEN_GIT_SHA"),
                env!("VERGEN_GIT_COMMIT_DATE"),
                if env!("VERGEN_GIT_DIRTY") == "true" {
                    "-dirty"
                } else {
                    ""
                }
            );
        }
    }
}

pub fn configure_panic_hook() {
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        default_panic(panic_info);
        error!("{}", panic_info);
        eprintln!("{}", panic_info);
        if let Some(location) = panic_info.location() {
            error!(
                "panic occurred in file '{}' at line {}",
                location.file(),
                location.line(),
            );
        } else {
            error!("panic occurred but can't get location information...");
        }
        process::exit(12);
    }));
}
