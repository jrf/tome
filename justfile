set shell := ["bash", "-uc"]

default:
    @just --list

build:
    cargo build

release:
    cargo build --release

run *args:
    cargo run -- {{args}}

check:
    cargo clippy
    cargo fmt --check

fmt:
    cargo fmt

clean:
    cargo clean

install: release
    @cp target/release/bm ~/.local/bin/bm
    @if [ "$(uname)" = "Darwin" ]; then codesign -s - ~/.local/bin/bm; fi
    @echo "Installed → ~/.local/bin/bm"
