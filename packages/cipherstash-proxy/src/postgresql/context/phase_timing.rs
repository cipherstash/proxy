use std::time::{Duration, Instant};

/// Tracks timing for individual phases of statement processing
#[derive(Clone, Debug, Default)]
pub struct PhaseTiming {
    /// SQL parsing and type-checking time
    pub parse_duration: Option<Duration>,
    /// Encryption operation time (includes ZeroKMS network)
    pub encrypt_duration: Option<Duration>,
    /// Time to write to PostgreSQL server
    pub server_write_duration: Option<Duration>,
    /// Time from server write to first response byte
    pub server_wait_duration: Option<Duration>,
    /// Time to receive complete server response
    pub server_response_duration: Option<Duration>,
    /// Time to write response to client
    pub client_write_duration: Option<Duration>,
    /// Decryption operation time
    pub decrypt_duration: Option<Duration>,
}

impl PhaseTiming {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record parse phase duration (first write wins)
    pub fn record_parse(&mut self, duration: Duration) {
        self.parse_duration.get_or_insert(duration);
    }

    /// Add parse duration (accumulate)
    pub fn add_parse(&mut self, duration: Duration) {
        self.parse_duration = Some(self.parse_duration.unwrap_or_default() + duration);
    }

    /// Record encrypt phase duration (first write wins)
    pub fn record_encrypt(&mut self, duration: Duration) {
        self.encrypt_duration.get_or_insert(duration);
    }

    /// Add encrypt duration (accumulate)
    pub fn add_encrypt(&mut self, duration: Duration) {
        self.encrypt_duration = Some(self.encrypt_duration.unwrap_or_default() + duration);
    }

    /// Record server write phase duration (first write wins)
    pub fn record_server_write(&mut self, duration: Duration) {
        self.server_write_duration.get_or_insert(duration);
    }

    /// Add server write duration (accumulate)
    pub fn add_server_write(&mut self, duration: Duration) {
        self.server_write_duration =
            Some(self.server_write_duration.unwrap_or_default() + duration);
    }

    /// Record server wait phase duration (first byte latency, first write wins)
    pub fn record_server_wait(&mut self, duration: Duration) {
        self.server_wait_duration.get_or_insert(duration);
    }

    /// Record server response phase duration (first write wins)
    pub fn record_server_response(&mut self, duration: Duration) {
        self.server_response_duration.get_or_insert(duration);
    }

    /// Add server response duration (accumulate)
    pub fn add_server_response(&mut self, duration: Duration) {
        self.server_response_duration =
            Some(self.server_response_duration.unwrap_or_default() + duration);
    }

    /// Record client write phase duration (first write wins)
    pub fn record_client_write(&mut self, duration: Duration) {
        self.client_write_duration.get_or_insert(duration);
    }

    /// Add client write duration (accumulate)
    pub fn add_client_write(&mut self, duration: Duration) {
        self.client_write_duration =
            Some(self.client_write_duration.unwrap_or_default() + duration);
    }

    /// Record decrypt phase duration (first write wins)
    pub fn record_decrypt(&mut self, duration: Duration) {
        self.decrypt_duration.get_or_insert(duration);
    }

    /// Add decrypt duration (accumulate)
    pub fn add_decrypt(&mut self, duration: Duration) {
        self.decrypt_duration = Some(self.decrypt_duration.unwrap_or_default() + duration);
    }

    /// Calculate total tracked duration
    pub fn total_tracked(&self) -> Duration {
        [
            self.parse_duration,
            self.encrypt_duration,
            self.server_write_duration,
            self.server_wait_duration,
            self.server_response_duration,
            self.client_write_duration,
            self.decrypt_duration,
        ]
        .iter()
        .filter_map(|d| *d)
        .sum()
    }
}

/// Helper to time a phase
pub struct PhaseTimer {
    start: Instant,
}

impl PhaseTimer {
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_timing_records_durations() {
        let mut timing = PhaseTiming::new();

        timing.record_parse(Duration::from_millis(5));
        timing.record_encrypt(Duration::from_millis(100));
        timing.record_server_wait(Duration::from_millis(50));

        assert_eq!(timing.parse_duration, Some(Duration::from_millis(5)));
        assert_eq!(timing.encrypt_duration, Some(Duration::from_millis(100)));
        assert_eq!(timing.server_wait_duration, Some(Duration::from_millis(50)));
    }

    #[test]
    fn total_tracked_sums_durations() {
        let mut timing = PhaseTiming::new();

        timing.record_parse(Duration::from_millis(5));
        timing.record_encrypt(Duration::from_millis(100));
        timing.record_server_wait(Duration::from_millis(50));

        assert_eq!(timing.total_tracked(), Duration::from_millis(155));
    }

    #[test]
    fn add_encrypt_accumulates() {
        let mut timing = PhaseTiming::new();

        timing.add_encrypt(Duration::from_millis(10));
        timing.add_encrypt(Duration::from_millis(15));

        assert_eq!(timing.encrypt_duration, Some(Duration::from_millis(25)));
    }

    #[test]
    fn add_server_write_accumulates() {
        let mut timing = PhaseTiming::new();

        timing.add_server_write(Duration::from_millis(3));
        timing.add_server_write(Duration::from_millis(7));

        assert_eq!(timing.server_write_duration, Some(Duration::from_millis(10)));
    }

    #[test]
    fn phase_timer_measures_elapsed() {
        let timer = PhaseTimer::start();
        std::thread::sleep(Duration::from_millis(10));
        let elapsed = timer.elapsed();

        assert!(elapsed >= Duration::from_millis(10));
    }
}
