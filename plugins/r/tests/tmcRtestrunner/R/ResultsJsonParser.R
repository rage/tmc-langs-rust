.create_json_run_results <- function(run_results) {
  json_test_results <- list()
  for (test_result in run_results$test_results) {
    json_test_results[[length(json_test_results) + 1]] <- .create_json_test_result(test_result)
  }
  json_run_results <- list("runStatus"   = jsonlite::unbox(run_results$run_status),
                           "backtrace"   = lapply(run_results$backtrace, jsonlite::unbox),
                           "testResults" = json_test_results)
  return(json_run_results)
}

#Creates JSON for each different test case.
.create_json_test_result <- function(test_result) {
  test_result <- list(status   = jsonlite::unbox(test_result$status),
                     name      = jsonlite::unbox(format(test_result$name)),
                     message   = jsonlite::unbox(test_result$message),
                     backtrace = lapply(test_result$backtrace, jsonlite::unbox),
                     points    = test_result$points)
  return(test_result)
}

#' @importFrom jsonlite toJSON
#' @importFrom jsonlite prettify

# Writes JSON based on the whole test or available points result.
.write_json <- function(results, filename) {
  # json utf-8 coded:
  json      <- jsonlite::toJSON(results, pretty = FALSE)
  json_utf8 <- jsonlite::prettify(enc2utf8(json))
  # encode json to UTF-8 and write to file called 'filename'
  write(json_utf8, file = filename)
}

# Prints results.
.print_results_from_json <- function(json_result) {
  for (test in json_result$testResults) {
    cat(sep = "", test$name, ": ", test$status, "\n")
    if (test$message != "") {
      cat(sep = "", "\n", test$message, "\n")
    }
  }
}
