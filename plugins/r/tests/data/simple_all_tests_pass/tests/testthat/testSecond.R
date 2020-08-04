library("testthat")

points_for_all_tests(c("r2"))

test("minus works", c("r2.1"), {
  expect_equal(minus(5, 2), 3)
})
