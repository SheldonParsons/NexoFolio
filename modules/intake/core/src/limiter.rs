//! One token bucket per `platform`, in process. Enough while the backend runs
//! as a single instance; a shared counter would replace it behind the same API.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};

/// At most this many platforms are tracked. Platforms are client-chosen
/// strings, so the map must not grow without bound.
const MAX_BUCKETS: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Sustained batches per minute and platform.
    pub rate_per_minute: u32,
    /// Batches a platform may send at once after being idle.
    pub burst: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            rate_per_minute: 120,
            burst: 30,
        }
    }
}

struct Bucket {
    tokens: f64,
    updated: DateTime<Utc>,
}

pub(crate) struct RateLimiter {
    per_second: f64,
    burst: f64,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl RateLimiter {
    pub(crate) fn new(limits: Limits) -> Self {
        Self {
            per_second: f64::from(limits.rate_per_minute.max(1)) / 60.0,
            burst: f64::from(limits.burst.max(1)),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Takes one token, or says how many whole seconds until one is available.
    pub(crate) fn take(&self, platform: &str, now: DateTime<Utc>) -> Result<(), u64> {
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if !buckets.contains_key(platform) && buckets.len() >= MAX_BUCKETS {
            self.forget_idle(&mut buckets, now);
        }
        let bucket = buckets.entry(platform.to_owned()).or_insert(Bucket {
            tokens: self.burst,
            updated: now,
        });
        bucket.tokens = self.refilled(bucket, now);
        bucket.updated = now.max(bucket.updated);
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            let wait = ((1.0 - bucket.tokens) / self.per_second).ceil();
            Err((wait as u64).max(1))
        }
    }

    fn refilled(&self, bucket: &Bucket, now: DateTime<Utc>) -> f64 {
        let elapsed = (now - bucket.updated).num_milliseconds().max(0) as f64 / 1000.0;
        (bucket.tokens + elapsed * self.per_second).min(self.burst)
    }

    /// A full bucket behaves exactly like a missing one, so dropping those is
    /// free. If every bucket is busy, start over rather than grow.
    fn forget_idle(&self, buckets: &mut HashMap<String, Bucket>, now: DateTime<Utc>) {
        buckets.retain(|_, bucket| self.refilled(bucket, now) < self.burst);
        if buckets.len() >= MAX_BUCKETS {
            buckets.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000 + seconds, 0).unwrap()
    }

    #[test]
    fn burst_then_steady_rate_per_platform() {
        let limiter = RateLimiter::new(Limits {
            rate_per_minute: 60,
            burst: 3,
        });
        for _ in 0..3 {
            assert_eq!(limiter.take("fetcher", at(0)), Ok(()));
        }
        assert_eq!(limiter.take("fetcher", at(0)), Err(1));
        assert_eq!(limiter.take("swagger", at(0)), Ok(()), "own bucket");
        assert_eq!(limiter.take("fetcher", at(1)), Ok(()), "one per second");
        assert_eq!(limiter.take("fetcher", at(1)), Err(1));
        assert_eq!(limiter.take("fetcher", at(1) + TimeDelta::hours(1)), Ok(()));
    }

    #[test]
    fn retry_after_rounds_up_to_whole_seconds() {
        let limiter = RateLimiter::new(Limits {
            rate_per_minute: 6,
            burst: 1,
        });
        assert_eq!(limiter.take("slow", at(0)), Ok(()));
        assert_eq!(limiter.take("slow", at(0)), Err(10));
        assert_eq!(limiter.take("slow", at(4)), Err(6));
    }

    #[test]
    fn many_platforms_do_not_grow_the_map() {
        let limiter = RateLimiter::new(Limits::default());
        for index in 0..MAX_BUCKETS * 2 {
            assert_eq!(limiter.take(&format!("p{index}"), at(0)), Ok(()));
        }
        assert!(limiter.buckets.lock().unwrap().len() <= MAX_BUCKETS);
    }
}
