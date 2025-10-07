#!/bin/bash

# Note: Asks for your tmc password in order to access all exercises and solutions
# Runs an integration test that downloads all the exercises (cached in ./test-cache) for various courses and verifies that
# 1. The tests fail on the template (some exercises are intended to pass on the template, so failing this is just a warning)
# 2. The tests pass on the example solution
# 3. The tests pass when the example solution is packaged like a submission and extracted over the template

cargo test test_policies_on_course_exercises -- --ignored --nocapture | tee test-cache/out.log
