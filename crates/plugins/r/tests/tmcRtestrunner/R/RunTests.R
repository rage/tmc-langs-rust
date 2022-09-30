#' @exportPattern "^[[:alpha:]]+"

#' @title Run tests from project directory
#'
#' @description Runs the tests from project directory and writes results
#' JSON to the root of the project as .results.json.
#'
#' @usage
#' run_tests(project_path = getwd(), print = FALSE,
#'           addin_data = NULL)
#'
#' @param project_path The absolute path to the root of the project being tested.
#' Default value is current work directory
#'
#' @param print Boolean that prints resulst if true. DEFAULT is FALSE.
#'
#' @param addin_data A named list with addin specific data or NULL, when
#' for separating server and addin. Default if NULL.
#'
#' @return List of result data. List keys: \code{runStatus} (string),
#' \code{backtrace} (list), \code{test_results} (list)
#'

# Args:
#  project_path: The absolute path to the root of the project being tested.
#  print: If TRUE, prints results; if not, not. DEFAULT is FALSE.
#
# Returns:
#   Run results list containing: runStatus (string), backtrace (list), test_results (list)

#' @export
run_tests <- function(project_path = getwd(), print = FALSE, addin_data = NULL) {
  # Runs tests for project and returns the results.
  # If sourcing_error occurs, .sourcing_error_run_result returns the results.
  if (is.null(addin_data)) {
    addin_data <- list(only_test_names = FALSE,
                       server_mode     = TRUE)
  }
  addin_data$print <- print
  run_results <- tryCatch({.run_tests_project(project_path, addin_data)},
                          sourcing_error = .sourcing_error_run_result,
                          run_error = .run_error_run_result)

  json_run_results <- .create_json_run_results(run_results)
  .write_json(json_run_results, file.path(project_path, ".results.json"))

  if (print) {
    cat("\n===  Test results  ============================== \n\n")
    .print_results_from_json(json_run_results)
  }
  invisible(run_results)
}

#' @importFrom testthat test_file

.run_tests_project <- function(project_path, addin_data) {
  test_results <- list()
  # Lists all the files in the path beginning with "test" and ending in ".R"
  test_files <- list.files(path       = file.path(project_path, "tests", "testthat"),
                           pattern    = "test.*\\.R$",
                           full.names = TRUE,
                           recursive  = FALSE)
  .GlobalEnv$points               <- list()
  .GlobalEnv$points_for_all_tests <- list()
  addin_data$test_files <- test_files
  test_env_list  <- .create_test_env(project_path, addin_data)
  for (ind in seq_along(test_files)) {
    # the main loop. This needs to be rewritten
    test_file <- test_files[ind]
    .GlobalEnv$points               <- list()
    .GlobalEnv$points_for_all_tests <- list()
    if (!addin_data$only_test_names) {
      test_env_l <- test_env_list[[ind]]
    } else {
      test_env_l <- list(env = test_env_list, error_msg = NULL)
    }
    file_results <- .run_tests_file(test_file, project_path, test_env_l)
    test_results <- c(test_results, file_results)
  }
  return(list(run_status   = "success",
              backtrace    = list(),
              test_results = test_results))
}

.run_tests_file <- function(file_path, project_path, test_env_l) {
  test_env <- test_env_l$env
  .define_tester_functions(test_env)
  test_file_output <- tryCatch({
    testthat::test_file(file_path,
                        reporter = "silent",
                        env = test_env)
  }, error = .signal_run_error)

  test_file_results <- .create_file_results(test_file_output,
                                            points,
                                            .GlobalEnv$points_for_all_tests,
                                                    # <--- FIX THIS
                                            test_env_l$error_msg)
  return(test_file_results)
}

.signal_sourcing_error <- function(error) {
  sourcing_error <- simpleError(message = error$message, call = error$call)
  class(sourcing_error) <- c("sourcing_error", class(sourcing_error))
  signalCondition(sourcing_error)
}

#' @importFrom jsonlite unbox

.sourcing_error_run_result <- function(sourcing_error) {
  cat("Sourcing tests failed:\n")
  cat("Error in ")
  cat(deparse(sourcing_error$call))
  cat(" : ")
  cat(sourcing_error$message)
  cat("\n")

  split_message <-
    strsplit(paste("Error in ", deparse(sourcing_error$call)," : ",
                   sourcing_error$message, sep = ""),
             split = "\n")
  backtrace <- lapply(split_message[[1]], unbox)
  return(list("run_status" = "sourcing_failed", "backtrace" = backtrace, "test_results" = list()))
}

.signal_run_error <- function(error) {
  run_error <- simpleError(message = error$message, call = error$call)
  class(run_error) <- c("run_error", class(run_error))
  signalCondition(run_error)
}

.run_error_run_result <- function(run_error) {
  cat("Runtime error.\n")
  split_message <- strsplit(run_error$message, split = "\n")
  backtrace     <- lapply(split_message[[1]], unbox)
  return(list("run_status"   = "run_failed",
              "backtrace"    = backtrace,
              "test_results" = list()))
}

