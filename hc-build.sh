#!/usr/bin/env bash

if cargo build --release; then
  echo build successful >&2
  sudo cp ./target/release/hc /usr/local/bin
else
    echo build FAILED >&2
fi
