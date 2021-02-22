//! Utility struct for printing progress reports.

use once_cell::sync::OnceCell;
use serde::Serialize;
use std::error::Error;
use std::time::Instant;
use std::{ops::DerefMut, sync::RwLock};
use type_map::concurrent::TypeMap;

pub struct ProgressReporter2 {
    reporters: TypeMap,
    current_progress: f64,
    total_steps_left: usize,
    start_time: Instant,
    stage_steps: Vec<usize>, // steps left
}

pub static PROGRESS_REPORTERS: OnceCell<RwLock<ProgressReporter2>> = OnceCell::new();

/// Subscribes to progress reports of type T with callback of type F.
/// Initializes progress reporters if necessary.
pub fn subscribe<T, F>(progress_report: F)
where
    T: 'static + Send + Sync,
    F: 'static + Sync + Send + Fn(StatusUpdate<T>) -> Result<(), DynError>,
{
    let lock = PROGRESS_REPORTERS.get_or_init(|| {
        RwLock::new(ProgressReporter2 {
            reporters: TypeMap::new(),
            current_progress: 0.0,
            total_steps_left: 0,
            start_time: Instant::now(),
            stage_steps: Vec::new(),
        })
    });
    let mut guard = lock.write().unwrap();
    let reporter = ProgressReporter {
        progress_report: Box::new(progress_report),
    };
    guard.reporters.insert(reporter);
}

/// Starts a new stage for the reporter associated with type T.
pub fn start_stage<T: 'static + Send + Sync>(total_steps: usize, message: String, data: Option<T>) {
    // check for init
    if let Some(lock) = PROGRESS_REPORTERS.get() {
        let mut reporter = lock.write().unwrap();
        let reporter = reporter.deref_mut();

        // check for subscriber
        if let Some(progress_reporter) = reporter.reporters.get::<ProgressReporter<'static, T>>() {
            reporter.total_steps_left += total_steps;
            reporter.stage_steps.push(total_steps);

            // report status
            let status_update = StatusUpdate {
                finished: false,
                message,
                percent_done: reporter.current_progress * 100.0,
                time: reporter.start_time.elapsed().as_millis(),
                data,
            };
            let _r = progress_reporter.progress_report.as_ref()(status_update);
        }
    }
}

/// Progresses the reporter associated with type T.
pub fn progress_stage<T: 'static + Send + Sync>(message: String, data: Option<T>) {
    // check for init
    if let Some(lock) = PROGRESS_REPORTERS.get() {
        let mut reporter = lock.write().unwrap();
        let reporter = reporter.deref_mut();

        // check for subscriber
        if let Some(progress_reporter) =
            reporter.reporters.get_mut::<ProgressReporter<'static, T>>()
        {
            // check for stage
            if let Some(stage_steps_left) = reporter.stage_steps.last_mut() {
                // check if steps left in stage
                if *stage_steps_left > 0 {
                    let step_progress =
                        (1.0 - reporter.current_progress) / reporter.total_steps_left as f64;
                    *stage_steps_left -= 1;
                    reporter.total_steps_left -= 1;
                    reporter.current_progress =
                        f64::min(reporter.current_progress + step_progress, 1.0);
                    // guard against going over 1.0
                }

                let status_update = StatusUpdate {
                    finished: false,
                    message,
                    percent_done: reporter.current_progress * 100.0,
                    time: reporter.start_time.elapsed().as_millis(),
                    data,
                };
                let _r = progress_reporter.progress_report.as_ref()(status_update);
            }
        }
    }
}

pub fn finish_stage<T: 'static + Send + Sync>(message: String, data: Option<T>) {
    // check for init
    if let Some(lock) = PROGRESS_REPORTERS.get() {
        let mut reporter = lock.write().unwrap();
        let reporter = reporter.deref_mut();

        // check for subscriber
        if let Some(progress_reporter) = reporter.reporters.get::<ProgressReporter<'static, T>>() {
            // check for stage
            if let Some(stage_steps_left) = reporter.stage_steps.pop() {
                let step_progress =
                    (1.0 - reporter.current_progress) / reporter.total_steps_left as f64;
                reporter.total_steps_left -= stage_steps_left;
                reporter.current_progress = f64::min(
                    reporter.current_progress + stage_steps_left as f64 * step_progress,
                    1.0,
                ); // guard against going over 1.0

                let status_update = StatusUpdate {
                    finished: true,
                    message,
                    percent_done: reporter.current_progress * 100.0,
                    time: reporter.start_time.elapsed().as_millis(),
                    data,
                };
                let _r = progress_reporter.progress_report.as_ref()(status_update);
            }
        }
    }
}

/// The format for all status updates. May contain some data.
#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct StatusUpdate<T> {
    pub finished: bool,
    pub message: String,
    pub percent_done: f64,
    pub time: u128,
    pub data: Option<T>,
}

// compatible with anyhow
type DynError = Box<dyn Error + Send + Sync + 'static>;
type UpdateClosure<'a, T> = dyn 'a + Sync + Send + Fn(StatusUpdate<T>) -> Result<(), DynError>;

/// The struct that keeps track of progress and contains a closure for reporting whenever progress is made.
struct ProgressReporter<'a, T> {
    progress_report: Box<UpdateClosure<'a, T>>,
}

#[cfg(test)]
mod test {
    use super::*;
    use std::sync::{Arc, Mutex, MutexGuard};

    static PROGRESS_MUTEX: OnceCell<Mutex<()>> = OnceCell::new();

    fn init() -> MutexGuard<'static, ()> {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();

        // wait for lock and clear reporter map
        let mutex = PROGRESS_MUTEX.get_or_init(|| Mutex::new(()));
        let guard = mutex.lock().unwrap();
        if let Some(reporters) = PROGRESS_REPORTERS.get() {
            let mut reporters = reporters.write().unwrap();
            *reporters = ProgressReporter2 {
                reporters: TypeMap::new(),
                current_progress: 0.0,
                total_steps_left: 0,
                start_time: Instant::now(),
                stage_steps: Vec::new(),
            };
        }
        guard
    }

    #[test]
    fn single_stage_progress() {
        let _lock = init();

        let su = Arc::new(Mutex::new(None));
        let suc = Arc::clone(&su);
        subscribe::<usize, _>(move |s| {
            log::debug!("got {:#?}", s);
            *suc.lock().unwrap() = Some(s);
            Ok(())
        });

        start_stage::<usize>(2, "starting".to_string(), None);

        progress_stage::<usize>("hello".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 50.0).abs() < 0.01);
        progress_stage::<usize>("hello!".to_string(), Some(2));
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 100.0).abs() < 0.01);
    }

    #[test]
    fn multi_stage_progress() {
        let _lock = init();

        let su = Arc::new(Mutex::new(None));
        let suc = Arc::clone(&su);
        subscribe::<usize, _>(move |s| {
            log::debug!("got {:#?}", s);
            *suc.lock().unwrap() = Some(s);
            Ok(())
        });

        start_stage::<usize>(2, "starting".to_string(), None);
        progress_stage::<usize>("msg".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 50.0).abs() < 0.01);

        start_stage::<usize>(2, "starting".to_string(), None);
        progress_stage::<usize>("msg".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 66.6666).abs() < 0.01);

        start_stage::<usize>(2, "starting".to_string(), None);
        progress_stage::<usize>("msg".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 74.9992).abs() < 0.01);
        progress_stage::<usize>("msg".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 83.3317).abs() < 0.01);
        finish_stage::<usize>("msg".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 83.3317).abs() < 0.01);

        finish_stage::<usize>("msg".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 91.6642).abs() < 0.01);

        finish_stage::<usize>("msg".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 100.0).abs() < 0.01);
    }
}
