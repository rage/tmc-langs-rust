library('testthat')

source("../../R/main.R")

points_for_all_tests(c("r1"))

test("RetTrue works.", c("r1.1"), {
  expect_true(ret_true())
})

test("RetOne works.", c("r1.2"), {
  expect_equal(ret_one(), 1)
})

test("Add works.", c("r1.3", "r1.4"), {
  expect_equal(add(1, 1), 2)
  expect_equal(add(0, 1), 1)
  expect_equal(add(0, 0), 0)
  expect_equal(add(5, 5), 10)
})
