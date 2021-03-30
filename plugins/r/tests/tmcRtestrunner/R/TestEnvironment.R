.create_test_env <- function(project_path) {
  test_env <- new.env()
  .define_tester_functions(test_env)
  tryCatch({.override_functions(test_env, project_path)},
           error = .signal_sourcing_error)
  tryCatch({.source_files(test_env, project_path)},
           error = .signal_sourcing_error)
  return (test_env)
}

.source_files <- function(test_env, project_path) {
  for (file in list.files(pattern = "[.]R$", path = paste0(project_path, "/R/"), full.names = TRUE)) {
    source(file, test_env, keep.source = getOption("keep.source"))
  }
}

.define_tester_functions <- function(test_env) {
  assign("points_for_all_tests",function(points) {
    .GlobalEnv$points_for_all_tests <- points
  },envir=test_env)
  lockBinding("points_for_all_tests",test_env)
  #The test that wraps around test_that()-method and stores the points
  #to global environment.
  assign("test",function(desc, points, code,timeout = 30) {
    .GlobalEnv$points[[desc]] <- points
    withTimeout({
      test_that(desc, code);
    },
    timeout = timeout);
  },envir=test_env)
  lockBinding("test",test_env)
}


.override_functions <- function(test_env, project_path) {
    mock_path <- paste(sep = .Platform$file.sep, project_path, "tests",
                        "testthat", "mock.R")
    if (file.exists(mock_path)) {
        sys.source(mock_path, test_env)
    }
}

# .source_from_test_file <- function(test_location, test_env) {
#   script_name <- basename(test_location)
#   script_name <- substr(script_name, 5, nchar(script_name))
#   source_folder <- "R/"
#   # Checks whether list is empty and if it is, modifies the first letter of the script to lower case.
#   if (length(list.files(path = source_folder, pattern = script_name, full.names = T, recursive = FALSE)) == 0) {
#     substr(script_name, 1, 1) <- tolower(substr(script_name, 1, 1))
#   }
#   sys.source(paste0(source_folder, script_name), test_env)
# }
#
# .create_test_env_file <- function(test_file) {
#   test_env <- new.env()
#   .define_tester_functions(test_env)
#   tryCatch({.source_from_test_file(test_file, test_env)},
#            error = .signal_sourcing_error)
#   return (test_env)
# }
