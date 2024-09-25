build:
    cargo build
    cargo build-sbf

lint:
    cargo clippy --no-deps --tests --features test-bpf

test-all:
    cargo build-sbf
    cargo test-sbf
    cargo test

test TEST_NAME:
    cargo build-sbf
    cargo test-sbf -- {{ TEST_NAME }}
