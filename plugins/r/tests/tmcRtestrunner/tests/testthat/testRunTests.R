test_resources_dir <- paste(sep = "", getwd(), "/resources")

#projects for testing:
simple_all_tests_pass_project_path <- paste(sep = "", test_resources_dir, "/simple_all_tests_pass")
simple_all_tests_pass_with_plot_project_path <- paste(sep = "",
                        test_resources_dir,
                        "/simple_all_tests_pass_with_plot")
simple_some_tests_fail_project_path <- paste(sep = "", test_resources_dir, "/simple_some_tests_fail")
simple_sourcing_fail_project_path <- paste(sep = "", test_resources_dir, "/simple_sourcing_fail")
simple_run_fail_project_path <- paste(sep = "", test_resources_dir, "/simple_run_fail")

test_that("Test pass in simple_all_tests_pass", {
  remove_old_results_json(simple_all_tests_pass_project_path)
  test_results <- .run_tests_project(simple_all_tests_pass_project_path)$test_results

  #All tests should pass:
  for (i in length(test_results)) {
    expect_equal(test_results[[i]]$status, "pass")
  }
})

#Tests that all exercise entrys store the point for all tests.
test_that("Tests that pass in simple_all_tests_pass all have the point for all tests", {
  remove_old_results_json(simple_all_tests_pass_project_path)
  test_results <- .run_tests_project(simple_all_tests_pass_project_path)$test_results
  point <- "r1"
  for (i in 1:3) {
    vec1 <- test_results[[i]]$points
    expect_true(point %in% vec1)
  }
})

test_that(".run_tests_project adds points accordingly for simple_all_tests_pass", {
  remove_old_results_json(simple_all_tests_pass_project_path)
  test_results <- .run_tests_project(simple_all_tests_pass_project_path)$test_results
  #"RetTrue works." points
  expect_equal(test_results[[1]]$points, c("r1", "r1.1"))
  #"RetOne works." points
  expect_equal(test_results[[2]]$points, c("r1", "r1.2"))
  #"Add works." points
  expect_equal(test_results[[3]]$points, c("r1", "r1.3", "r1.4"))
})

test_that("run_tests create .results.json", {
  remove_old_results_json(simple_all_tests_pass_project_path)
  run_tests(simple_all_tests_pass_project_path)
  expect_true(file.exists(paste(sep = "", simple_all_tests_pass_project_path, "/.results.json")))
})

test_that("Not all tests pass in simple_some_tests_fail.", {
  remove_old_results_json(simple_some_tests_fail_project_path)
  test_results <- .run_tests_project(simple_some_tests_fail_project_path)$test_results

  #"RetTrue works." should pass
  expect_equal(test_results[[1]]$status, "pass")
  #"RetOne works." should pass
  expect_equal(test_results[[2]]$status, "pass")
  #"Add works." should pass
  expect_equal(test_results[[3]]$status, "pass")
  #"RetFalse returns true" should FAIL
  expect_equal(test_results[[4]]$status, "fail")
  #"RetTrue works but there asre no points." should pass
  expect_equal(test_results[[5]]$status, "pass")
})

test_that("run_results returns and writes.results.json as expected for simple_some_tests_fail", {
  remove_old_results_json(simple_some_tests_fail_project_path)

  run_results <- run_tests(simple_some_tests_fail_project_path)
  test_results <- run_results$test_results

  results_json <- read_json(paste(sep = "", simple_some_tests_fail_project_path, "/.results.json"))
  test_results_json <- results_json$testResults

  #expected results for simple_some_tests_fail
  expected_test_result <- list()
  expected_test_result[[1]] <- list(status = "pass", name = "ret_true works.",
                                    message = "", backtrace = list(), points = list("r1", "r1.1"))
  expected_test_result[[2]] <- list(status = "pass", name = "ret_one works.",
                                    message = "", backtrace = list(), points = list("r1", "r1.2"))
  expected_test_result[[3]] <- list(status = "pass", name = "add works.",
                                    message = "", backtrace = list(), points = list("r1", "r1.3", "r1.4"))
  #expected backtrace for 4th test:
  backtrace_test4 <- list(paste0("1: expect_true(ret_false()) in ", simple_some_tests_fail_project_path,
                           "/tests/testthat/testMain.R#21"))
  expected_test_result[[4]] <- list(status = "fail", name = "ret_false returns true",
                                    message = "ret_false() isn't true.",
                                    backtrace = backtrace_test4, points = list("r1", "r1.5"))
  expected_test_result[[5]] <- list(status = "pass", name = "ret_true works but there are no points.",
                                    message = "", backtrace = list(), points = list("r1"))

  #runStatus should be true and backtrace empty for .results.json
  expect_equal(results_json$runStatus, "success")
  expect_equal(results_json$backtrace, list())

  #testResults is as expected for .results.json
  for (i in 1:5) expect_equal(test_results_json[[i]], expected_test_result
                          [[i]])

  #runStatus should be true and backtrace empty
  expect_equal("success", run_results$run_status)
  expect_equal(list(), run_results$backtrace)

  #test_results returns as expected
  for (i in 1:5) {
    expect_equal(test_results[[i]]$status, expected_test_result[[i]]$status)
    expect_equal(test_results[[i]]$name, expected_test_result[[i]]$name)
    expect_equal(test_results[[i]]$message, expected_test_result[[i]]$message)
    expect_equal(test_results[[i]]$backtrace, expected_test_result[[i]]$backtrace)
    expect_equal(as.list(test_results[[i]]$points), expected_test_result[[i]]$points)
  }
})

test_that("RunTests does print on print = TRUE", {
  remove_old_results_json(simple_all_tests_pass_project_path)
  #simple_all_tests_pass prints as expected
  expect_output(run_tests(simple_all_tests_pass_project_path, print = TRUE),
                "ret_true works.: pass\nret_one works.: pass\nadd works.: pass")
})

test_that("RunTests doesn't print on print = FALSE", {
  remove_old_results_json(simple_all_tests_pass_project_path)
  expect_silent(run_tests(simple_all_tests_pass_project_path, print = FALSE))
})

test_that("Sourcing fail handled accordingly.", {
  remove_old_results_json(simple_sourcing_fail_project_path)

  run_tests(simple_sourcing_fail_project_path)
  results_json <- read_json(paste(sep = "", simple_sourcing_fail_project_path, "/.results.json"))

  #runStatus whould be "sourcing_failed", backtrace empty and testResults empty
  expect_equal(results_json$runStatus, "sourcing_failed")
  expect_equal(results_json$testResults, list())

  #Backtrace should contain correct error:
  expect_true(grepl(":7:9: unexpected 'in'",results_json$backtrace[[1]]))
  expect_equal("6: ret_one <- function() {", results_json$backtrace[[2]])
  expect_equal("7:   error in", results_json$backtrace[[3]])
  expect_equal("           ^", results_json$backtrace[[4]])
})

test_that("Run fail handled accordingly.", {
  remove_old_results_json(simple_run_fail_project_path)

  run_tests(simple_run_fail_project_path)
  results_json <- read_json(paste(sep = "", simple_run_fail_project_path, "/.results.json"))

  #runStatus whould be "run_fail" and testResults empty
  expect_equal(results_json$runStatus, "run_failed")
  expect_equal(results_json$testResults, list())

  #Backtrace should contain correct error:
  expect_true(grepl(":9:3: unexpected 'in'",results_json$backtrace[[1]]))
  expect_equal("8:   #Produces run fail:", results_json$backtrace[[2]])
  expect_equal("9:   in", results_json$backtrace[[3]])
  expect_equal("     ^", results_json$backtrace[[4]])
})

test_that("Test pass with overriden functions", {
  remove_old_results_json(simple_all_tests_pass_with_plot_project_path)
  test_results <- .run_tests_project(simple_all_tests_pass_with_plot_project_path)$test_results
  #All tests should pass:
  for (i in length(test_results)) {
    expect_equal(test_results[[i]]$status, "pass")
  }
})
