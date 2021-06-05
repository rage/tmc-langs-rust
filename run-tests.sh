#! /bin/bash
node ./bindings/tmc-langs-node/jest/mock_server.js &
NODE_ID=$!
trap 'kill $NODE_ID' EXIT

cargo test && \
  npm --prefix bindings/tmc-langs-node/ run jest
