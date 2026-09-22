CHECK_CFLAGS=$(shell pkg-config --cflags check)
CHECK_LDFLAGS=$(shell pkg-config --libs check)
SRC_FILES=test_source.c tmc-check.c checkhelp.c ../src/source.c

all: test

# -O0 because an optimising build (nix's cc-wrapper adds -O2) elides the leak in two().
test: $(SRC_FILES)
	gcc $(CHECK_CFLAGS) -O0 -g -Wall -Wextra -std=c99 -o $@ $(SRC_FILES) $(CHECK_LDFLAGS) -lm

clean:
	rm -f test tmc_available_points.txt tmc_test_results.xml valgrind.log mockinput mockoutput

run: test
	./test

run-valgrind: test
	valgrind --track-origins=yes --leak-check=full --log-file=valgrind.log ./test

get-points:
	./test --print-available-points
