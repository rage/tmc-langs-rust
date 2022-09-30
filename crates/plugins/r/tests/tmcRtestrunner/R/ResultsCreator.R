.create_file_results <- function(testthat_file_output, tests_points,
                                 file_points, error_message) {

  results <- list()
  for (test in testthat_file_output) {
    name <- test$test
    status <- .get_status_for_test(test)
    message <- .create_message_for_test(test, status, error_message)
    backtrace <- .create_backtrace_for_test(test, status, error_message)
    points <- .get_points_for_test(name,
                                   tests_points,
                                   file_points)

    test_result <- list("name" = name,
                        "status" = status,
                        "points" = points,
                        "message" = message,
                        "backtrace" = backtrace)

    results[[length(results) + 1]] <- test_result
  }
  return(results)
}

.get_points_for_test <- function(test_name, tests_points, file_points) {
  if (is.null(tests_points[[test_name]])) {
    test_points <- vector()
  } else {
    test_points <- tests_points[[test_name]]
  }
  test_points <- c(file_points, test_points)
  return(test_points)
}

.get_status_for_test <- function(test) {
  if (.check_if_test_passed(test)) {
    status <- "pass"
  } else {
    status <- "fail"
  }
  return(status)
}

#Checks if a single test passed
.check_if_test_passed <- function(test) {
  ret <- TRUE
  for (result in test$results) {
    if (!.check_if_result_passed(result)) {
      ret <- FALSE
      break
    }
  }
  return(ret)
}

#Check if a single result passed
.check_if_result_passed <- function(result) {
  return(format(result) == "As expected")
}

.message_from_failed_result <- function(result, error_message) {
  message_rows <- strsplit(result$message, "\n")[[1]]
  if (!is.null(error_message) & !is.null(result$trace)) {
    error_message_rows <- strsplit(error_message, "\n")[[1]]
    message_rows <- c(message_rows, "", "\033[32mPossibly due to error:\033[39m", error_message_rows)
  }
  return(paste(message_rows, collapse = "\n"))
}

.create_message_for_test <- function(test, status, error_message) {
  if (status == "pass") return("")

  for (result in test$results) {
    if (format(result) != "As expected") {
      return(.message_from_failed_result(result, error_message))
    }
  }
  return("")
}

.create_backtrace_for_test <- function(testthat_test_result, status, error_message) {
  if (status == "pass") return(list())

  for (result in testthat_test_result$results) {
    if (format(result) != "As expected") {
      backtrace <- list()
      i <- 1;
# this would be the correct parser, but this trace is not wanted.
# Later this will be just removed
#      for (call in result$trace$calls) {
      for (call in result$calls) {
        backtrace <- append(backtrace, paste0(i, ": ", .create_call_message(call)))
        i <- i + 1
      }
      return(backtrace)
    }
  }
  return(list())

}

.create_call_message <- function(call) {
  call_str <- format(call)
  call_srcref <- attributes(call)$srcref
  srcref_data <- c(call_srcref)
  srcfile_filename <- attributes(call_srcref)$srcfile$filename

  if (is.null(call_srcref)) {
    message <- paste0(call_str)
  } else {
    message <- paste0(call_str, " in ", srcfile_filename, "#", srcref_data[[1]])
  }

  return(message)
}
