/// Utility struct for printing progress reports.
use serde::Serialize;
use std::cell::Cell;
use std::error::Error;
use std::time::Instant;

/// The format for all status updates. May contain some data.
#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct StatusUpdate<T> {
    pub finished: bool,
    pub message: String,
    pub percent_done: f64,
    pub time: Option<u128>,
    pub data: Option<T>,
}

// compatible with anyhow
type DynError = Box<dyn Error + Send + Sync + 'static>;
type UpdateClosure<'a, T> = Box<dyn 'a + Fn(StatusUpdate<T>) -> Result<(), DynError>>;

pub struct ProgressReporter<'a, T> {
    progress_report: UpdateClosure<'a, T>,
    progress_steps_total: Cell<usize>,
    progress_steps_done: Cell<usize>,
    start_time: Cell<Option<Instant>>,
}

impl<'a, T> ProgressReporter<'a, T> {
    /// Takes a closure that will be called with all status updates, for example to print it.
    pub fn new(progress_report: impl 'a + Fn(StatusUpdate<T>) -> Result<(), DynError>) -> Self {
        Self {
            progress_report: Box::new(progress_report),
            progress_steps_total: Cell::new(1),
            progress_steps_done: Cell::new(0),
            start_time: Cell::new(None),
        }
    }

    /// Increase the total amount of steps in the process being reported.
    ///
    /// Should be incremented to its final value before the process starts.
    pub fn increment_progress_steps(&self, amount: usize) {
        self.progress_steps_total
            .set(self.progress_steps_total.get() + amount);
    }

    /// Starts the timer if not started yet.
    pub fn start_timer(&self) {
        if self.start_time.get().is_none() {
            self.start_time.set(Some(Instant::now()))
        }
    }

    /// Progress the current step to the percent given.
    pub fn progress(
        &self,
        message: String,
        step_percent_done: f64,
        data: Option<T>,
    ) -> Result<(), DynError> {
        self.start_timer();

        let from_prev_steps = self.progress_steps_done.get() as f64;
        let percent_done =
            (from_prev_steps + step_percent_done) / self.progress_steps_total.get() as f64;

        self.progress_report.as_ref()(StatusUpdate {
            finished: false,
            message,
            percent_done,
            time: self.start_time.get().map(|t| t.elapsed().as_millis()),
            data,
        })
    }

    /// Finish the current step and the whole process if the current step is the last one.
    pub fn finish_step(&self, message: String, data: Option<T>) -> Result<(), DynError> {
        self.progress_steps_done
            .set(self.progress_steps_done.get() + 1);
        if self.progress_steps_done.get() == self.progress_steps_total.get() {
            // all done
            let result = self.progress_report.as_ref()(StatusUpdate {
                finished: true,
                message,
                percent_done: 1.0,
                time: self.start_time.take().map(|t| t.elapsed().as_millis()),
                data,
            });
            result
        } else {
            // more steps left, next step at 0%
            self.progress(message, 0.0, None)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn multi_step_progress() {
        init();

        let su = Arc::new(Mutex::new(None));
        let report = ProgressReporter::new(|s| {
            log::debug!("got {:#?}", s);
            *su.lock().unwrap() = Some(s);
            Ok(())
        });

        report.increment_progress_steps(2);

        report.progress("msg".to_string(), 0.2, Some(1)).unwrap();
        assert!(
            (su.lock().unwrap().as_ref().unwrap().percent_done - (0.2 / 3.0)).abs() < f64::EPSILON
        );
        report.progress("msg".to_string(), 0.8, Some(2)).unwrap();
        assert!(
            (su.lock().unwrap().as_ref().unwrap().percent_done - (0.8 / 3.0)).abs() < f64::EPSILON
        );
        report.finish_step("msg".to_string(), Some(3)).unwrap();
        assert!(
            (su.lock().unwrap().as_ref().unwrap().percent_done - (1.0 / 3.0)).abs() < f64::EPSILON
        );
        report.finish_step("msg".to_string(), Some(4)).unwrap();
        assert!(
            (su.lock().unwrap().as_ref().unwrap().percent_done - (2.0 / 3.0)).abs() < f64::EPSILON
        );
        report.progress("msg".to_string(), 0.5, Some(5)).unwrap();
        assert!(
            (su.lock().unwrap().as_ref().unwrap().percent_done - (2.5 / 3.0)).abs() < f64::EPSILON
        );
        report.finish_step("msg".to_string(), None).unwrap();
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 1.0).abs() < f64::EPSILON);
    }
}
