pub mod chunk;
pub mod datadog;
pub mod domain;
pub mod openai;
pub mod planner;
pub mod qdrant;
pub mod rag_service;
pub mod reranker;

#[cfg(test)]
pub(crate) mod test_support {
    use std::ffi::OsString;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    pub(crate) fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env test mutex poisoned")
    }

    pub(crate) struct EnvVarGuard {
        key: &'static str,
        value: Option<OsString>,
    }

    impl EnvVarGuard {
        pub(crate) fn preserve(key: &'static str) -> Self {
            Self {
                key,
                value: std::env::var_os(key),
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.value {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }
}
