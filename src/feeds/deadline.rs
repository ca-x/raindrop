use std::future::Future;

use tokio::time::{Instant, timeout_at};

pub(super) struct StrictDeadlineElapsed;

pub(super) async fn strict_timeout_at<F>(
    deadline: Instant,
    future: F,
) -> Result<F::Output, StrictDeadlineElapsed>
where
    F: Future,
{
    if Instant::now() >= deadline {
        return Err(StrictDeadlineElapsed);
    }
    let output = timeout_at(deadline, future)
        .await
        .map_err(|_| StrictDeadlineElapsed)?;
    if Instant::now() >= deadline {
        Err(StrictDeadlineElapsed)
    } else {
        Ok(output)
    }
}
