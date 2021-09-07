test_resources_dir <- paste(sep = "", getwd(), "/resources")

#projects for testing:
simple_all_tests_fail_project_path <- paste(sep = "", test_resources_dir, "/simple_all_tests_fail")
simple_all_tests_pass_project_path <- paste(sep = "", test_resources_dir, "/simple_all_tests_pass")
simple_some_tests_fail_project_path <- paste(sep = "", test_resources_dir, "/simple_some_tests_fail")

test_that("First test in all passing testMain returns correct points", {
  #All tests should return true:
  test_points <- .get_available_points(simple_all_tests_pass_project_path)
  points <- list()
  points <- test_points[["ret_true works."]]
  expect_true("r1" %in% points)
  expect_true("r1.1" %in% points)
  expect_true(!("r2" %in% points))
})

test_that("Second test in all passing testMain returns correct points", {
  test_points <- .get_available_points(simple_all_tests_pass_project_path)
  points <- test_points[["ret_one works."]]
  expect_true("r1" %in% points)
  expect_true("r1.2" %in% points)
  expect_true(!("r2" %in% points))
})

test_that("Third test in all passing testMain returns correct points", {
  test_points <- .get_available_points(simple_all_tests_pass_project_path)
  points <- test_points[["add works."]]
  expect_true("r1" %in% points)
  expect_true("r1.3" %in% points)
  expect_true("r1.4" %in% points)
  expect_true(!("r2" %in% points))
})

test_that("First test in all passing testMain returns correct points", {
  test_points <- .get_available_points(simple_all_tests_pass_project_path)
  points <- test_points[["minus works"]]
  expect_true("r2" %in% points)
  expect_true("r2.1" %in% points)
  expect_true(!("r1" %in% points))
})


test_that("run_available_points works and runs available_points", {
  remove_old_available_points_json(simple_all_tests_pass_project_path)

  ##Call run_available_points
  run_available_points(simple_all_tests_pass_project_path)

  ##Get the path to the supposed file.
  available_points_path <- paste(sep = "", simple_all_tests_pass_project_path, "/.available_points.json")

  #Check that the file exists
  expect_equal(T, file.exists(available_points_path))
})

test_that("/.available_points.json has correct values", {
  remove_old_available_points_json(simple_all_tests_pass_project_path)

  ##Call run_available_points
  run_available_points(simple_all_tests_pass_project_path)

  ##Get the path to the supposed file.
  available_points_path <- paste(sep = "", simple_all_tests_pass_project_path, "/.available_points.json")

  #Create json-object from .available_points.json.
  json <- read_json(available_points_path)

  #Test that json has correct values.
  expect_equal(names(json)[[1]], "ret_true works.")
  expect_true(length(json[[1]]) > 0)
})
