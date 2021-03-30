test_resources_dir <- paste(sep = "", getwd(), "/resources")

#projects for testing:
simple_all_tests_pass_project_path <- paste(sep = "", test_resources_dir, "/simple_all_tests_pass")
simple_all_tests_pass_project_path_with_plot <- paste(sep = "",
                                test_resources_dir,
                                "/simple_all_tests_pass_with_plot")

test_that("test_env is created correctly for simple_all_tests_pass", {
  test_env <- .create_test_env(simple_all_tests_pass_project_path)

  #Test functions should exist:
  expect_true(exists("test", where = test_env, mode = "function"))
  expect_true(exists("points_for_all_tests", where = test_env, mode = "function"))

  #Functions from main.R and second.R should exist:
  expect_true(exists("ret_true", where = test_env, mode = "function"))
  expect_true(exists("ret_one", where = test_env, mode = "function"))
  expect_true(exists("add", where = test_env, mode = "function"))
  expect_true(exists("minus", where = test_env, mode = "function"))
})

test_that("plot is overwriten in test_env", {
    test_env <- .create_test_env(simple_all_tests_pass_project_path_with_plot)
    mock_path <- paste(sep = .Platform$file.sep,
                    simple_all_tests_pass_project_path_with_plot,
                    "tests", "testthat", "mock.R")

    expect_true(file.exists(mock_path))
    expect_true(exists("plot", where = test_env, mode = "function"))
    expect_true(exists("used_plot_args", where = test_env))
    expect_true(exists("paste", where = test_env, mode = "function"))
    expect_true(exists("used_paste_args", where = test_env))
})
