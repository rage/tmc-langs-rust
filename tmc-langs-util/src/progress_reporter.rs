/// Utility struct for printing progress reports.
use serde::Serialize;
use std::error::Error;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
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
type UpdateClosure<'a, T> = Box<dyn 'a + Sync + Send + Fn(StatusUpdate<T>) -> Result<(), DynError>>;

/// The reporter contains a RefCell for the timer, meaning care should be taken when using in a multithreaded context.
/// The first call to progress and each step completion should be called from one thread. Other progress calls can be done from separate threads.
pub struct ProgressReporter<'a, T> {
    progress_report: UpdateClosure<'a, T>,
    progress_steps_total: AtomicUsize,
    progress_steps_done: AtomicUsize,
    start_time: Mutex<Option<Instant>>,
}

impl<'a, T> ProgressReporter<'a, T> {
    /// Takes a closure that will be called with all status updates, for example to print it.
    pub fn new(
        progress_report: impl 'a + Sync + Send + Fn(StatusUpdate<T>) -> Result<(), DynError>,
    ) -> Self {
        Self {
            progress_report: Box::new(progress_report),
            progress_steps_total: AtomicUsize::new(1),
            progress_steps_done: AtomicUsize::new(0),
            start_time: Mutex::new(None),
        }
    }

    /// Increase the total amount of steps in the process being reported.
    ///
    /// Should be incremented to its final value before the process starts.
    pub fn increment_progress_steps(&self, amount: usize) {
        self.progress_steps_total
            .fetch_add(amount, Ordering::SeqCst);
    }

    /// Starts the timer if not started yet.
    pub fn start_timer(&self) {
        let mut start_time = self.start_time.lock().unwrap();
        if start_time.is_none() {
            *start_time = Some(Instant::now())
        }
    }

    /// Progress the current step to the percent given.
    pub fn progress(
        &self,
        message: impl ToString,
        step_percent_done: f64,
        data: Option<T>,
    ) -> Result<(), DynError> {
        self.start_timer();

        let from_prev_steps = self.progress_steps_done.load(Ordering::SeqCst) as f64;
        let percent_done = (from_prev_steps + step_percent_done)
            / self.progress_steps_total.load(Ordering::SeqCst) as f64;

        let start_time = self.start_time.lock().unwrap();
        self.progress_report.as_ref()(StatusUpdate {
            finished: false,
            message: message.to_string(),
            percent_done,
            time: start_time.map(|t| t.elapsed().as_millis()),
            data,
        })
    }

    /// Finish the current step and the whole process if the current step is the last one.
    pub fn finish_step(&self, message: impl ToString, data: Option<T>) -> Result<(), DynError> {
        self.progress_steps_done.fetch_add(1, Ordering::SeqCst);
        if self.progress_steps_done.load(Ordering::SeqCst)
            == self.progress_steps_total.load(Ordering::SeqCst)
        {
            // all done
            let mut start_time = self.start_time.lock().unwrap();
            let result = self.progress_report.as_ref()(StatusUpdate {
                finished: true,
                message: message.to_string(),
                percent_done: 1.0,
                time: start_time.take().map(|t| t.elapsed().as_millis()),
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
