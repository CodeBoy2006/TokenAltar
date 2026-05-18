use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

#[derive(Clone, Default)]
pub struct MetricsState {
    rolling_hour_tokens: Arc<AtomicI64>,
}

impl MetricsState {
    pub fn add_tokens(&self, tokens: i64) {
        self.rolling_hour_tokens
            .fetch_add(tokens, Ordering::Relaxed);
    }

    pub fn tokens_last_hour(&self) -> i64 {
        self.rolling_hour_tokens.load(Ordering::Relaxed)
    }
}
