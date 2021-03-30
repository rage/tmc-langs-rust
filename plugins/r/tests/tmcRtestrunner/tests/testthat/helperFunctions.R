.run_create_file_result_for_files <- function(project_path) {
  test_results <- list()

  test_files <- list.files(path = file.path(project_path, "tests", "testthat"), pattern = "test.*\\.R",
                           full.names = T, recursive = FALSE)
  for (test_file in test_files) {
    .GlobalEnv$points <- list()
    .GlobalEnv$points_for_all_tests <- list()
    test_env <- .create_test_env(project_path)
    test_results<- c(test_results, .create_file_results(test_file(test_file, reporter = "silent", env = test_env),
                                                                   .GlobalEnv$points,
                                                                  .GlobalEnv$points_for_all_tests))
  }

  return(test_results)
}

remove_old_results_json <- function(project_path) {
  results_json_path <- paste(sep = "", project_path, "/.results.json")
  if (file.exists(results_json_path)) {
    file.remove(results_json_path)
  }
}

remove_old_available_points_json <- function(project_path) {
  available_points_json_path <- paste(sep = "", project_path, "/.available_points.json")
  if (file.exists(available_points_json_path)) {
    file.remove(available_points_json_path)
  }
}
