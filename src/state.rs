use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const SURGE_WINDOW: Duration = Duration::from_secs(60 * 60);
const SURGE_BUCKET: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct MetricsState {
    rolling_hour_tokens: Arc<Mutex<RollingTokenWindow>>,
}

impl Default for MetricsState {
    fn default() -> Self {
        Self {
            rolling_hour_tokens: Arc::new(Mutex::new(RollingTokenWindow::default())),
        }
    }
}

impl MetricsState {
    pub fn add_tokens(&self, tokens: i64) {
        self.add_tokens_at(tokens, Instant::now());
    }

    pub fn tokens_last_hour(&self) -> i64 {
        self.tokens_last_hour_at(Instant::now())
    }

    fn add_tokens_at(&self, tokens: i64, now: Instant) {
        self.rolling_hour_tokens
            .lock()
            .expect("metrics window mutex poisoned")
            .add(tokens, now);
    }

    fn tokens_last_hour_at(&self, now: Instant) -> i64 {
        self.rolling_hour_tokens
            .lock()
            .expect("metrics window mutex poisoned")
            .total(now)
    }
}

#[derive(Default)]
struct RollingTokenWindow {
    buckets: VecDeque<TokenBucket>,
}

struct TokenBucket {
    started_at: Instant,
    tokens: i64,
}

impl RollingTokenWindow {
    fn add(&mut self, tokens: i64, now: Instant) {
        self.prune(now);
        if let Some(last) = self.buckets.back_mut()
            && now
                .checked_duration_since(last.started_at)
                .is_some_and(|age| age < SURGE_BUCKET)
        {
            last.tokens += tokens;
            return;
        }
        self.buckets.push_back(TokenBucket {
            started_at: now,
            tokens,
        });
    }

    fn total(&mut self, now: Instant) -> i64 {
        self.prune(now);
        self.buckets.iter().map(|bucket| bucket.tokens).sum()
    }

    fn prune(&mut self, now: Instant) {
        while self
            .buckets
            .front()
            .and_then(|bucket| now.checked_duration_since(bucket.started_at))
            .is_some_and(|age| age >= SURGE_WINDOW)
        {
            self.buckets.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_window_drops_tokens_outside_last_hour() {
        let metrics = MetricsState::default();
        let now = Instant::now();

        metrics.add_tokens_at(100, now - Duration::from_secs(61 * 60));
        metrics.add_tokens_at(40, now - Duration::from_secs(30 * 60));
        metrics.add_tokens_at(2, now);

        assert_eq!(metrics.tokens_last_hour_at(now), 42);
    }

    #[test]
    fn rolling_window_groups_minute_bucket() {
        let metrics = MetricsState::default();
        let now = Instant::now();

        metrics.add_tokens_at(10, now);
        metrics.add_tokens_at(15, now + Duration::from_secs(30));

        assert_eq!(
            metrics.tokens_last_hour_at(now + Duration::from_secs(30)),
            25
        );
    }
}
