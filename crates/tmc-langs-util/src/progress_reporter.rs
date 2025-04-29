//! Utility struct for printing progress reports.

use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::{ops::DerefMut, sync::RwLock, time::Instant};
use type_map::concurrent::TypeMap;

/// The format for all status updates. May contain some data.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct StatusUpdate<T> {
    pub finished: bool,
    pub message: String,
    pub percent_done: f64,
    pub time: u32,
    pub data: Option<T>,
}

// the closure called to report progress, could for example print the report as JSON
type UpdateClosure<T> = dyn 'static + Sync + Send + Fn(StatusUpdate<T>);

/// The struct that keeps track of progress for a given progress update type T and contains a closure for reporting whenever progress is made.
struct ProgressReporter<T> {
    progress_report: Box<UpdateClosure<T>>,
}

/// Contains all the different progress reporters and keeps track of the overall progress.
struct ProgressReporterContainer {
    reporters: TypeMap,
    current_progress: f64,
    total_steps_left: u32,
    start_time: Instant,
    stage_steps: Vec<u32>, // steps left
}

impl ProgressReporterContainer {
    pub fn elapsed_millis(&self) -> u32 {
        // for the time to not fit into a u32, the time elapsed would have to be over a month
        // which isn't going to happen here
        self.start_time.elapsed().as_millis() as u32
    }
}

static PROGRESS_REPORTERS: OnceCell<RwLock<ProgressReporterContainer>> = OnceCell::new();

/// Subscribes to progress reports of type T with callback of type F called every time progress is made with type T.
pub fn subscribe<T, F>(progress_report: F)
where
    T: 'static + Send + Sync,
    F: 'static + Sync + Send + Fn(StatusUpdate<T>),
{
    let lock = PROGRESS_REPORTERS.get_or_init(|| {
        RwLock::new(ProgressReporterContainer {
            reporters: TypeMap::new(),
            current_progress: 0.0,
            total_steps_left: 0,
            start_time: Instant::now(),
            stage_steps: Vec::new(),
        })
    });
    let mut guard = lock
        .write()
        .expect("only fails if the lock is poisoned; we should never panic while holding the lock");
    let reporter = ProgressReporter {
        progress_report: Box::new(progress_report),
    };
    guard.reporters.insert(reporter);
}

/// Starts a new stage.
pub fn start_stage<T: 'static + Send + Sync>(total_steps: u32, message: String, data: Option<T>) {
    // check for init
    if let Some(lock) = PROGRESS_REPORTERS.get() {
        let mut reporter = lock.write().expect(
            "only fails if the lock is poisoned; we should never panic while holding the lock",
        );
        let reporter = reporter.deref_mut();
        reporter.total_steps_left += total_steps;
        reporter.stage_steps.push(total_steps);

        // check for subscriber
        if let Some(progress_reporter) = reporter.reporters.get::<ProgressReporter<T>>() {
            // report status
            let status_update = StatusUpdate {
                finished: false,
                message,
                percent_done: reporter.current_progress,
                time: reporter.elapsed_millis(),
                data,
            };
            progress_reporter.progress_report.as_ref()(status_update);
        }
    }
}

/// Progresses the current stage.
pub fn progress_stage<T: 'static + Send + Sync>(message: String, data: Option<T>) {
    // check for init
    if let Some(lock) = PROGRESS_REPORTERS.get() {
        let mut reporter = lock.write().expect(
            "only fails if the lock is poisoned; we should never panic while holding the lock",
        );
        let reporter = reporter.deref_mut();

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

            // check for subscriber
            let time = reporter.elapsed_millis();
            if let Some(progress_reporter) = reporter.reporters.get_mut::<ProgressReporter<T>>() {
                let status_update = StatusUpdate {
                    finished: false,
                    message,
                    percent_done: reporter.current_progress,
                    time,
                    data,
                };
                progress_reporter.progress_report.as_ref()(status_update);
            }
        }
    }
}

/// Finishes the current stage.
pub fn finish_stage<T: 'static + Send + Sync>(message: String, data: Option<T>) {
    // check for init
    if let Some(lock) = PROGRESS_REPORTERS.get() {
        let mut reporter = lock.write().expect(
            "only fails if the lock is poisoned; we should never panic while holding the lock",
        );
        let reporter = reporter.deref_mut();

        // check for stage
        if let Some(stage_steps_left) = reporter.stage_steps.pop() {
            let step_progress =
                (1.0 - reporter.current_progress) / reporter.total_steps_left as f64;
            reporter.total_steps_left -= stage_steps_left;
            reporter.current_progress = f64::min(
                reporter.current_progress + stage_steps_left as f64 * step_progress,
                1.0,
            ); // guard against going over 1.0

            // check for subscriber
            if let Some(progress_reporter) = reporter.reporters.get::<ProgressReporter<T>>() {
                let status_update = StatusUpdate {
                    finished: true,
                    message,
                    percent_done: reporter.current_progress,
                    time: reporter.elapsed_millis(),
                    data,
                };
                progress_reporter.progress_report.as_ref()(status_update);
            }
        }

        // All of the stages have been finished, resetting progress for future events.
        if reporter.total_steps_left == 0 && (reporter.current_progress - 1.0_f64).abs() < 0.001 {
            reporter.current_progress = 0.0;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
            *reporters = ProgressReporterContainer {
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
        subscribe::<u32, _>(move |s| {
            log::debug!("got {s:#?}");
            *suc.lock().unwrap() = Some(s);
        });

        start_stage::<u32>(2, "starting".to_string(), None);

        progress_stage::<u32>("hello".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 0.5000).abs() < 0.01);
        progress_stage::<u32>("hello!".to_string(), Some(2));
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 1.0000).abs() < 0.01);
    }

    #[test]
    fn multi_stage_progress() {
        let _lock = init();

        let su = Arc::new(Mutex::new(None));
        let suc = Arc::clone(&su);
        subscribe::<u32, _>(move |s| {
            log::debug!("got {s:#?}");
            *suc.lock().unwrap() = Some(s);
        });

        start_stage::<u32>(2, "starting".to_string(), None);
        progress_stage::<u32>("msg".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 0.5000).abs() < 0.01);

        start_stage::<u32>(2, "starting".to_string(), None);
        progress_stage::<u32>("msg".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 0.6666).abs() < 0.01);

        start_stage::<u32>(2, "starting".to_string(), None);
        progress_stage::<u32>("msg".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 0.7499).abs() < 0.01);
        progress_stage::<u32>("msg".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 0.8333).abs() < 0.01);
        finish_stage::<u32>("msg".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 0.8333).abs() < 0.01);

        finish_stage::<u32>("msg".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 0.9166).abs() < 0.01);

        finish_stage::<u32>("msg".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 1.0000).abs() < 0.01);
    }

    #[test]
    fn consecutive_progress() {
        let _lock = init();

        let su = Arc::new(Mutex::new(None));
        let suc = Arc::clone(&su);
        subscribe::<u32, _>(move |s| {
            log::debug!("got {s:#?}");
            *suc.lock().unwrap() = Some(s);
        });

        start_stage::<u32>(3, "starting".to_string(), None);
        progress_stage::<u32>("hello".to_string(), None);
        assert!(
            (su.lock().unwrap().as_ref().unwrap().percent_done - (1.0000 / 3.0000)).abs() < 0.01
        );
        progress_stage::<u32>("hello!".to_string(), Some(2));
        assert!(
            (su.lock().unwrap().as_ref().unwrap().percent_done - (2.0000 / 3.0000)).abs() < 0.01
        );
        finish_stage::<u32>("finished".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 1.0000).abs() < 0.01);

        start_stage::<u32>(2, "starting".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 0.0000).abs() < 0.01);
        progress_stage::<u32>("hello".to_string(), None);
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 0.5000).abs() < 0.01);
        progress_stage::<u32>("hello!".to_string(), Some(2));
        assert!((su.lock().unwrap().as_ref().unwrap().percent_done - 1.0000).abs() < 0.01);
    }
}
