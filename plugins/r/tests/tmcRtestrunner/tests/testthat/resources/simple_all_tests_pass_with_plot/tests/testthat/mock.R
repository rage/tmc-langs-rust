#In this file, the teacher can override R functions so that she/he can
#test whether the student has used some function (and maybe with the correct
#arguments)

used_plot_args <- list()
used_paste_args <- list()

plot <- function(x, y, ...) {
    params <- list(x = x, y = y, ...)

    # Assigning to environment before this function call:
    env_parent <- parent.frame()
    env_parent$used_plot_args[[length(used_plot_args) + 1]] <- params

    graphics::plot(x = x,y = y, ...)

    if (file.exists("Rplots.pdf")) {
      file.remove("Rplots.pdf")
    }
}

paste0 <- function(...) {
    params <- list(...)

    # Assigning to environment before this function call:
    env_parent <- parent.frame()
    env_parent$used_paste_args[[length(used_paste_args) + 1]] <- params

    base::paste0(...)
}
