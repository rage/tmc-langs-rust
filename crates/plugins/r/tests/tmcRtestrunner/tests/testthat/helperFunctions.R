.run_create_file_result_for_files <- function(project_path) {
  addin_data <- list(only_test_names = FALSE,
                     server_mode     = TRUE)
  test_results <- list()

  test_files <- list.files(path = file.path(project_path, "tests", "testthat"),
                           pattern = "test.*\\.R$",
                           full.names = TRUE,
                           recursive = FALSE)
  .GlobalEnv$points <- list()
  .GlobalEnv$points_for_all_tests <- list()
  addin_data$test_files <- test_files
  test_env_list <- .create_test_env(project_path, addin_data)
  for (ind in seq_along(test_files)) {
    test_file <- test_files[[ind]]
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
