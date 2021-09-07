
.get_available_points <- function(project_path) {
  .init_global_vars()
  test_files <- list.files(path = paste0(project_path, "/tests/testthat"), pattern = "test.*\\.R",
                                         full.names = T, recursive = FALSE)
  for (test_file in test_files) {
    .GlobalEnv$map_to_desc[[.GlobalEnv$counter]] <- list()
    .GlobalEnv$file_points[[.GlobalEnv$counter]] <- list()
    test_file(test_file, reporter = "silent", env = .create_counter_env(project_path))
    .GlobalEnv$counter <- .GlobalEnv$counter + 1
  }
  return (.add_points(.GlobalEnv$test_available_points, .GlobalEnv$file_points, .GlobalEnv$map_to_desc))
}

.init_global_vars <- function() {
  .GlobalEnv$test_available_points <- list()
  .GlobalEnv$file_points <- list()
  .GlobalEnv$map_to_desc <- list()
  .GlobalEnv$counter <- 1
}

.add_points <- function(test_available_points, file_points, map_to_desc) {
  all_available_points <- list()
  for (i in (1:unlist(.GlobalEnv$counter - 1))) {
    for (desc in map_to_desc[[i]]) {
      all_available_points[[desc]] <- c(file_points[[i]], test_available_points[[desc]])
    }
  }
  return (all_available_points)
}

.create_counter_env <- function(project_path) {
  test_env <- new.env()
  .define_counter_functions(test_env, project_path)
  return (test_env)
}

.define_counter_functions <- function(test_env, project_path) {
  .source_files(test_env, project_path)
  test_env$test <- function(desc, point, code){
    if (!(desc %in% .GlobalEnv$test_available_points)) {
      .GlobalEnv$test_available_points[[desc]] <- list()
    }
    .GlobalEnv$test_available_points[[desc]] <- c(point)
    .GlobalEnv$map_to_desc[[.GlobalEnv$counter]] <- c(.GlobalEnv$map_to_desc[[.GlobalEnv$counter]], desc)
  }
  test_env$points_for_all_tests <- function(points){
    .GlobalEnv$file_points[[.GlobalEnv$counter]] <- c(points)
  }
}

# Checks the available points for all test in the project without running test. Creates
# file .available_points.json in the project root.
run_available_points <- function(project_path = getwd()) {
  available_points <- .get_available_points(project_path)

  json_results <- .create_available_points_json_results(available_points)
  .write_json(json_results, paste0(project_path, "/.available_points.json"))
}
