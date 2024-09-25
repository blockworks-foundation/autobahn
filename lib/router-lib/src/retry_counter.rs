use std::time::Duration;

pub struct RetryCounter {
    error_count: u64,
    max_retry_count: u64,
    delay: Duration,
}

impl RetryCounter {
    pub fn new(max_retry_count: u64, delay: Duration) -> Self {
        RetryCounter {
            max_retry_count,
            delay,
            error_count: 0,
        }
    }

    pub async fn sleep(&self) {
        tokio::time::sleep(self.delay).await
    }

    pub fn reset(&mut self) {
        self.error_count = 0;
    }

    /// true if should retry, false if should bail
    pub fn on_new_error(&mut self) -> bool {
        self.error_count += 1;
        self.error_count <= self.max_retry_count
    }

    pub fn fail_or_ignore<T>(&mut self, result: anyhow::Result<T>) -> anyhow::Result<Option<T>> {
        match result {
            Err(e) => match self.on_new_error() {
                true => Ok(None),
                false => {
                    self.reset();
                    Err(e)
                }
            },
            Ok(v) => {
                self.reset();
                Ok(Some(v))
            }
        }
    }
}

#[macro_export]
macro_rules! fail_or_retry {
    ($retry_counter:expr, $f:expr) => {{
        loop {
            let result = $retry_counter.fail_or_ignore($f);
            match result {
                Ok(Some(value)) => {
                    break Ok(value);
                }
                Ok(None) => {
                    $retry_counter.sleep().await;
                    continue;
                }
                Err(e) => {
                    break Err(e);
                }
            }
        }
    }};
}
