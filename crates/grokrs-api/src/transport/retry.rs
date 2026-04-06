use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Configuration for retry behavior on transient errors.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_retries: u32,
    /// Base delay in milliseconds before the first retry.
    pub base_delay_ms: u64,
    /// Maximum delay in milliseconds between retries.
    pub max_delay_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_ms: 500,
            max_delay_ms: 30_000,
        }
    }
}

/// Determine whether a request should be retried based on the HTTP status code
/// and the current attempt number.
///
/// Returns `Some(delay)` if the request should be retried after the given
/// duration, or `None` if the request should not be retried.
///
/// Only retries on:
/// - 429 (Too Many Requests)
/// - 503 (Service Unavailable)
///
/// Uses exponential backoff with deterministic jitter derived from system time
/// to prevent thundering herd effects.
#[must_use]
pub fn should_retry(status: u16, attempt: u32, config: &RetryConfig) -> Option<Duration> {
    // Only retry on specific status codes
    if status != 429 && status != 503 {
        return None;
    }

    // Don't retry if we've exhausted attempts
    if attempt >= config.max_retries {
        return None;
    }

    // Exponential backoff: base * 2^attempt
    let exp_delay_ms = config.base_delay_ms.saturating_mul(1u64 << attempt.min(20));

    // Deterministic jitter based on system time to avoid thundering herd.
    // We use the low bits of the current time in nanoseconds as a simple
    // pseudo-random source. This is NOT cryptographically secure, but we
    // only need approximate randomness to spread out retry timing.
    let jitter_seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .subsec_nanos() as u64;

    // Jitter range: 50% to 150% of base jitter component
    // base_jitter = attempt * 100ms
    let base_jitter_ms = (attempt as u64).saturating_add(1).saturating_mul(100);
    // Scale factor: 0.5 + (seed % 1000) / 1000.0, giving range [0.5, 1.5)
    let scale_permille = 500 + (jitter_seed % 1000);
    let jitter_ms = base_jitter_ms.saturating_mul(scale_permille) / 1000;

    let total_ms = exp_delay_ms.saturating_add(jitter_ms);
    let clamped_ms = total_ms.min(config.max_delay_ms);

    Some(Duration::from_millis(clamped_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> RetryConfig {
        RetryConfig {
            max_retries: 3,
            base_delay_ms: 100,
            max_delay_ms: 10_000,
        }
    }

    #[test]
    fn retries_on_429() {
        let result = should_retry(429, 0, &test_config());
        assert!(result.is_some(), "should retry on 429");
    }

    #[test]
    fn retries_on_503() {
        let result = should_retry(503, 0, &test_config());
        assert!(result.is_some(), "should retry on 503");
    }

    #[test]
    fn does_not_retry_on_400() {
        let result = should_retry(400, 0, &test_config());
        assert!(result.is_none(), "should not retry on 400");
    }

    #[test]
    fn does_not_retry_on_401() {
        let result = should_retry(401, 0, &test_config());
        assert!(result.is_none(), "should not retry on 401");
    }

    #[test]
    fn does_not_retry_on_404() {
        let result = should_retry(404, 0, &test_config());
        assert!(result.is_none(), "should not retry on 404");
    }

    #[test]
    fn does_not_retry_on_500() {
        let result = should_retry(500, 0, &test_config());
        assert!(result.is_none(), "should not retry on 500");
    }

    #[test]
    fn does_not_retry_on_200() {
        let result = should_retry(200, 0, &test_config());
        assert!(result.is_none(), "should not retry on 200");
    }

    #[test]
    fn does_not_retry_beyond_max_retries() {
        let config = test_config();
        let result = should_retry(429, config.max_retries, &config);
        assert!(result.is_none(), "should not retry past max");
    }

    #[test]
    fn backoff_increases_with_attempt() {
        let config = RetryConfig {
            max_retries: 5,
            base_delay_ms: 100,
            max_delay_ms: 100_000,
        };

        // Run multiple times to account for jitter and take minimums
        let mut delays = Vec::new();
        for attempt in 0..4 {
            let delay = should_retry(429, attempt, &config).unwrap();
            delays.push(delay);
        }

        // With base=100ms, the exponential component alone is:
        // attempt 0: 100ms, attempt 1: 200ms, attempt 2: 400ms, attempt 3: 800ms
        // Even with jitter, later delays should generally be larger.
        // We verify the exponential component is increasing by checking that
        // later delays are meaningfully larger than earlier ones.
        assert!(
            delays[2].as_millis() > delays[0].as_millis(),
            "delay at attempt 2 ({:?}) should exceed delay at attempt 0 ({:?})",
            delays[2],
            delays[0]
        );
    }

    #[test]
    fn backoff_respects_max_delay() {
        let config = RetryConfig {
            max_retries: 30,
            base_delay_ms: 1000,
            max_delay_ms: 5000,
        };
        let delay = should_retry(429, 20, &config).unwrap();
        assert!(
            delay.as_millis() <= 5000,
            "delay should be clamped to max_delay_ms, got {:?}",
            delay
        );
    }

    #[test]
    fn default_retry_config_is_reasonable() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert!(config.base_delay_ms > 0);
        assert!(config.max_delay_ms > config.base_delay_ms);
    }
}
