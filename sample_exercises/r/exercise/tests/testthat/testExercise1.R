library("testthat")

points_for_all_tests(character(0))

# a)
test("exercise 1a is solved correctly", character(0), {
  expect_equal(a, c(1, 3, 4))
})

# b)
test("exercise 1b is solved correctly", character(0), {
  expect_equal(b, c(1, 9, 16))
})

# c)
A_correct <- matrix(1:9, nrow=3, byrow=FALSE)
test("exercise 1c is solved correctly", character(0), {
  expect_equal(A, A_correct)
})

# d)
A2_correct <- A_correct[,-3]
test("exercise 1d is solved correctly", character(0), {
  expect_equal(A2, A2_correct)
})

# e)
test("exercise 1e is solved correctly", character(0), {
  expect_equal(v, c(1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21))
})

# f)
test("exercise 1f is solved correctly", character(0), {
  expect_equal(s, 121)
})

# g)
test("exercise 1g is solved correctly", character(0), {
  expect_equal(v2, c(3, 9, 15, 21))
})