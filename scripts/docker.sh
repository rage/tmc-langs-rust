#!/bin/sh

# Runs the command given as arguments in a docker container, e.g.
# sh scripts/docker.sh cargo test
# The special argument `interactive` starts an interactive shell session inside the container

CMD="${*:-cargo build && cp /build/target/debug/tmc-langs-cli /build/out/}"
export DOCKER_BUILDKIT=1
docker build . -f docker/Dockerfile -t tmc-langs-rust
if [ "$CMD" = "interactive" ]; then
    docker run --ulimit nofile=1024:1024 -it --rm -v "$PWD":/build/out tmc-langs-rust bash
else
    docker run --ulimit nofile=1024:1024 --rm -v "$PWD":/build/out tmc-langs-rust bash -c "$CMD"
fi;
