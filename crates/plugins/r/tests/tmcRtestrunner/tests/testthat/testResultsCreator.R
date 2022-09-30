test_resources_dir <- paste(sep = "", getwd(), "/resources")

# projects for testing:
simple_all_tests_fail_project_path  <- paste(sep = "", test_resources_dir, "/simple_all_tests_fail")
simple_all_tests_pass_project_path  <- paste(sep = "", test_resources_dir, "/simple_all_tests_pass")
simple_some_tests_fail_project_path <- paste(sep = "", test_resources_dir, "/simple_some_tests_fail")

test_that("Test is reported to pass correctly", {
  test_output <- .run_create_file_result_for_files(simple_all_tests_pass_project_path)
  # All tests should return true:
  for (test in test_output) {
    expect_equal(test$status, "pass")
  }
})

test_that("Test is reported to fail correctly", {
  test_output <- .run_create_file_result_for_files(simple_all_tests_fail_project_path)
  # All tests should return false
  for (test in test_output) {
    expect_equal(test$status, "fail")
  }
})

test_that("Points are added correctly", {
  expected_result <- c("r1.1", "r1.2", "r1")
  name <- "point_testing"
  points <- list()
  points[[name]] <- c("r1.1", "r1.2")
  file_point <- c("r1")
  added_points <- .get_points_for_test(name, points, file_point)
  for (point in expected_result) {
    expect_equal(point %in% added_points, TRUE)
  }
  expect_equal(length(expected_result), length(added_points))
})

test_that("Status is added correctly for passing tests", {
  test_output <- .run_create_file_result_for_files(simple_all_tests_pass_project_path)
  # All tests should be given pass:
  for (test in test_output) {
    expect_equal(test$status, "pass")
  }
})

test_that("Status is added correctly for failing tests", {
  test_output <- .run_create_file_result_for_files(simple_all_tests_fail_project_path)
  for (test in test_output) {
    expect_equal(test$status, "fail")
  }
})

test_that("A message is given if the test fails", {
  test_output <- .run_create_file_result_for_files(simple_all_tests_fail_project_path)
  for (test in test_output) {
    expect_true(test$message != "")
  }
})

test_that("A message is not given if the test passes", {
  test_output <- .run_create_file_result_for_files(simple_all_tests_pass_project_path)
  for (test in test_output) {
    expect_true(test$message == "")
  }
})

test_that("A result is created correctly with status", {
  results <- .run_create_file_result_for_files(simple_all_tests_pass_project_path)
  for (result in results) {
    expect_equal(result$status, "pass")
  }
})

test_that("A result is created correctly with name", {
  results <- .run_create_file_result_for_files(simple_all_tests_pass_project_path)
  expect_equal(results[[1]]$name, "ret_true works.")
  expect_equal(results[[2]]$name, "ret_one works.")
  expect_equal(results[[3]]$name, "add works.")
})
