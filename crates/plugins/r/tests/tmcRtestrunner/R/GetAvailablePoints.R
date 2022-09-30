globalVariables(c("points"))

.container_init <- function() {
  container <- new.env(parent = emptyenv())
  .GlobalEnv$container <- container
}
.map_to_desc_add <- function(val) {
  .map_to_desc_set(c(.map_to_desc_ref(), val))
}
.map_to_desc_reset <- function() {
  .map_to_desc_set(list())
}
.map_to_desc_set <- function(val) {
  .GlobalEnv$container$map_to_desc <- val
}
.map_to_desc_ref <- function() {
  .GlobalEnv$container$map_to_desc
}
.file_points_ref <- function() {
  .GlobalEnv$container$file_points
}
.file_points_reset <- function() {
  .file_points_set(list())
}
.file_points_set <- function(val) {
  .GlobalEnv$container$file_points <- val
}
.test_available_points_init <- function() {
  .GlobalEnv$container$test_available_points <- list()
}
.test_available_points_ref <- function(idx) {
  .GlobalEnv$container$test_available_points[[idx]]
}
.test_available_points_set <- function(idx, val) {
  .GlobalEnv$container$test_available_points[[idx]] <- val
}

#' @importFrom testthat test_file

.get_available_points <- function(project_path) {
  .init_global_vars()
  all_available_points <- list()
  test_files <- list.files(path = paste0(project_path, "/tests/testthat"),
			   pattern = "test.*\\.R$",
			   full.names = TRUE,
			   recursive = FALSE)
  env <- .create_point_fetching_env(project_path)
  for (test_file in test_files) {
    .map_to_desc_reset()
    .file_points_reset()
    testthat::test_file(test_file, reporter = "silent", env = env)
    for (desc in .map_to_desc_ref()) {
      all_available_points[[desc]] <- c(.file_points_ref(),
					.test_available_points_ref(desc))
    }
  }
  .remove_global_vars()
  return (all_available_points)
}

.remove_global_vars <- function() {
  rm("container", envir = .GlobalEnv)
}

.init_global_vars <- function() {
  .container_init()
  .test_available_points_init()
  .file_points_reset()
  .map_to_desc_reset()
}

.create_point_fetching_env <- function(project_path) {
  test_env <- new.env()
  test_env$test <- function(desc, point, code){
    .test_available_points_set(desc, point)
    .map_to_desc_add(desc)
  }
  test_env$points_for_all_tests <- function(points){
    .file_points_set(points)
  }
  return (test_env)
}


#' @title Checks the available point for all tests
#'
#' @description Checks the available points for all test in the project
#' without running test. Creates file .available_points.json in the
#' project root.
#'
#' @usage run_available_points(project_path = getwd())
#'
#' @param project_path The absolute path to the root of the project being tested.
#' Default value is current work directory
#'
#' @return The function does not return values
#'


# Checks the available points for all test in the project without running test. Creates
# file .available_points.json in the project root.
#' @export
run_available_points <- function(project_path = getwd()) {
  results <- .get_available_points(project_path)
  .write_json(results, paste0(project_path, "/.available_points.json"))
}
