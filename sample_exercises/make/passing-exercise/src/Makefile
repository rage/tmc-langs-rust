SRC_FILES=main.c source.c

all: main

main: $(SRC_FILES)
	gcc -g -Wall -Wextra -pedantic -std=c99 -o $@ $(SRC_FILES) -lm

clean:
	rm -f main

run: main
	# Running our main function from file main.c
	./main
