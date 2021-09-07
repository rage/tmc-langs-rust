library("testthat")

points_for_all_tests(c("r1"))

test("ret_true works", c("r1.1"), {
  expect_true(ret_false())
})

test("ret_one works", c("r1.2"), {
  expect_equal(ret_one(), 2)
})

test("add works", c("r1.3", "r1.4"), {
  expect_equal(add(1, 1), 3)
  expect_equal(add(0, 1), 0)
  expect_equal(add(0, 0), 1)
  expect_equal(add(5, 5), 11)
})
