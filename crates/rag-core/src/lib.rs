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
    use std::sync::{Mutex, MutexGuard, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    pub(crate) fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env test mutex poisoned")
    }
}
