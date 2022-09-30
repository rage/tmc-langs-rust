library('testthat')

points_for_all_tests(c("r1"))

test("sin(x) plot called", c("r1.1"), {
    expect_true(exists("used_plot_args"))
    expect_false(is.null(used_plot_args[[1]]$x))
    expect_false(is.null(used_plot_args[[1]]$y))
    expect_false(is.null(used_plot_args[[1]]$main))
    expect_equal(used_plot_args[[1]]$x, seq(0,10, by = .1))
    expect_equal(used_plot_args[[1]]$y, sin(seq(0,10,by=.1)))
    expect_equal(used_plot_args[[1]]$main, "sin x")
})
