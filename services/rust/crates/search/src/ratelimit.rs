//! Client-side rate limiter honoring the provider organization limit
//! (10 requests per second; `.env.example` keeps headroom at 8 by default).
//!
//! Requests are spaced evenly rather than bucketed: the limiter reserves the
//! next slot under a `std` mutex and sleeps outside the lock, so it is safe
//! to share across tasks without holding a lock across `.await`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct RateLimiter {
    state: Mutex<LimiterState>,
    interval: Duration,
}

struct LimiterState {
    next_allowed: Instant,
}

impl RateLimiter {
    pub fn new(max_rps: u64) -> Self {
        let interval = Duration::from_millis(1_000 / max_rps.max(1));
        Self {
            state: Mutex::new(LimiterState {
                next_allowed: Instant::now(),
            }),
            interval,
        }
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Wait until the next call is allowed, then reserve the following slot.
    pub async fn acquire(&self) {
        loop {
            let wait = {
                let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
                let now = Instant::now();
                if state.next_allowed <= now {
                    state.next_allowed = now + self.interval;
                    return;
                }
                state.next_allowed - now
            };
            tokio::time::sleep(wait).await;
        }
    }

    /// Push the next allowed call out after a 429 / Retry-After response.
    pub fn penalize(&self, seconds: u64) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let candidate = Instant::now() + Duration::from_secs(seconds.max(1));
        if candidate > state.next_allowed {
            state.next_allowed = candidate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_respects_provider_limit() {
        assert_eq!(RateLimiter::new(8).interval(), Duration::from_millis(125));
        assert_eq!(RateLimiter::new(10).interval(), Duration::from_millis(100));
        // Degenerate input never produces a zero interval that would spin.
        assert!(RateLimiter::new(0).interval() >= Duration::from_millis(1));
    }

    #[test]
    fn penalize_pushes_next_allowed_into_the_future() {
        let limiter = RateLimiter::new(8);
        limiter.penalize(5);
        let state = limiter.state.lock().unwrap();
        assert!(state.next_allowed > Instant::now() + Duration::from_secs(4));
    }

    #[test]
    fn penalize_never_moves_the_deadline_backwards() {
        let limiter = RateLimiter::new(8);
        limiter.penalize(30);
        let first = limiter.state.lock().unwrap().next_allowed;
        limiter.penalize(1);
        let second = limiter.state.lock().unwrap().next_allowed;
        assert!(second >= first);
    }
}
