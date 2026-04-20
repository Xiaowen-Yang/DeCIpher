use std::time::Duration;

/// Retry configuration for provider HTTP requests.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub retryable_status_codes: Vec<u16>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            retryable_status_codes: vec![408, 429, 500, 502, 503, 504],
        }
    }
}

impl RetryConfig {
    /// Check whether a given HTTP status code is retryable.
    pub fn is_retryable(&self, status: u16) -> bool {
        self.retryable_status_codes.contains(&status)
    }

    /// Calculate delay for the given attempt (0-indexed).
    /// Uses exponential backoff: base_delay * 2^attempt, capped at max_delay.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let delay = self.base_delay * 2u32.saturating_pow(attempt);
        delay.min(self.max_delay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_retryable_codes() {
        let config = RetryConfig::default();
        assert!(config.is_retryable(429));
        assert!(config.is_retryable(500));
        assert!(config.is_retryable(503));
        assert!(!config.is_retryable(400));
        assert!(!config.is_retryable(401));
    }

    #[test]
    fn exponential_backoff() {
        let config = RetryConfig::default();
        assert_eq!(config.delay_for_attempt(0), Duration::from_secs(1));
        assert_eq!(config.delay_for_attempt(1), Duration::from_secs(2));
        assert_eq!(config.delay_for_attempt(2), Duration::from_secs(4));
        assert_eq!(config.delay_for_attempt(5), Duration::from_secs(30)); // capped
        assert_eq!(config.delay_for_attempt(10), Duration::from_secs(30)); // still capped
    }
}
