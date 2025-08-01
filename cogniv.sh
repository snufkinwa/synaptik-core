#!/bin/bash

BIN_PATH="./target/release/synaptik-core"

if [ ! -f "$BIN_PATH" ]; then
  echo "Binary not found. Run 'cargo build --release' first."
  exit 1
fi

$BIN_PATH "$@"
