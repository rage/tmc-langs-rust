# Runs the tests from project directory and writes results JSON to the root of the project
# as .results.json.
#
# Args:
#  project_path: The absolute path to the root of the project being tested.
#  print: If TRUE, prints results; if not, not. DEFAULT is FALSE.
#
# Returns:
#   Run results list containing: runStatus (string), backtrace (list), test_results (list)
run_tests <- function(project_path = getwd(), print = FALSE) {
  #Runs tests for project and returns the results.
  #If sourcing_error occurs, .sourcing_error_run_results returns the results.
  run_results <- tryCatch({.run_tests_project(project_path)},
                          sourcing_error = .sourcing_error_run_result,
                          run_error = .run_error_run_result)

  json_run_results <- .create_json_run_results(run_results)
  .write_json(json_run_results, file.path(project_path, ".results.json"))

  if (print) {
    .print_results_from_json(json_run_results)
  }

  invisible(run_results)
}

.run_tests_project <- function(project_path) {
  test_results <- list()
  #Lists all the files in the path beginning with "test" and ending in ".R"
  test_files <- list.files(path = file.path(project_path, "tests", "testthat"), pattern = "test.*\\.R",
                           full.names = TRUE, recursive = FALSE)

  for (test_file in test_files) {
    file_results <- .run_tests_file(test_file, project_path)
    test_results <- c(test_results, file_results)
  }
  return(list("run_status" = "success", "backtrace" = list(), "test_results" = test_results))
}

.run_tests_file <- function(file_path, project_path) {
  .GlobalEnv$points <- list()
  .GlobalEnv$points_for_all_tests <- list()

  test_env = .create_test_env(project_path)
  test_file_output <- tryCatch({test_file(file_path, reporter = "silent", env = test_env)},
                               error = .signal_run_error)

  test_file_results <- .create_file_results(test_file_output, points, .GlobalEnv$points_for_all_tests)

  return(test_file_results)
}

.signal_sourcing_error <- function(error) {
  sourcing_error <- simpleError(message = error$message, call = error$call)
  class(sourcing_error) <- c("sourcing_error", class(sourcing_error))
  signalCondition(sourcing_error)
}

.sourcing_error_run_result <- function(sourcing_error) {
  split_message <- strsplit(sourcing_error$message, split = "\n")
  backtrace <- lapply(split_message[[1]], unbox)
  return(list("run_status" = "sourcing_failed", "backtrace" = backtrace, "test_results" = list()))
}

.signal_run_error <- function(error) {
  run_error <- simpleError(message = error$message, call = error$call)
  class(run_error) <- c("run_error", class(run_error))
  signalCondition(run_error)
}

.run_error_run_result <- function(run_error) {
  split_message <- strsplit(run_error$message, split = "\n")
  backtrace <- lapply(split_message[[1]], unbox)
  return(list("run_status" = "run_failed", "backtrace" = backtrace, "test_results" = list()))
}
