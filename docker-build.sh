#!/bin/sh

export DOCKER_BUILDKIT=1
docker build . -f docker/Dockerfile -t tmc-langs-rust
docker run --rm -v "$PWD":/build/out tmc-langs-rust bash -c "cargo build && cp /build/target/debug/tmc-langs-cli /build/out/"
