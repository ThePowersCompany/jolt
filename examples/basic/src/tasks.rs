use std::time::{Duration, Instant};

use joltr_core::{Task, TaskFuture, TaskScheduler};

const STATS_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) fn scheduler() -> TaskScheduler {
    let mut scheduler = TaskScheduler::new();
    scheduler.register(StatsTask::default());
    scheduler
}

struct StatsTask {
    started_at: Instant,
    runs: u64,
}

impl Default for StatsTask {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            runs: 0,
        }
    }
}

impl Task for StatsTask {
    fn name(&self) -> &str {
        "basic-example-stats"
    }

    fn interval(&self) -> Duration {
        STATS_INTERVAL
    }

    fn run(&mut self) -> TaskFuture<'_> {
        Box::pin(async move {
            self.runs += 1;
            println!(
                "basic-example stats: uptime_secs={} runs={}",
                self.started_at.elapsed().as_secs(),
                self.runs
            );
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_registers_stats_task() {
        let scheduler = scheduler();

        assert_eq!(scheduler.len(), 1);
        assert_eq!(
            scheduler.get(0).expect("task registered").name(),
            "basic-example-stats"
        );
    }

    #[test]
    fn stats_task_interval_is_30_seconds() {
        assert_eq!(StatsTask::default().interval(), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn stats_task_logs_and_counts_runs() {
        let mut task = StatsTask::default();

        task.run().await.expect("stats task succeeds");
        task.run().await.expect("stats task succeeds again");

        assert_eq!(task.runs, 2);
    }
}
