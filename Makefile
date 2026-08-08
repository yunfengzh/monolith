# $env:RUST_BACKTRACE=1; cargo run --example stump
default:
	cargo run --example rhai

debug:
	rust-gdb target/debug/examples/crossref

# $env:RUST_TEST_NOCAPTURE=1; cargo test --lib
# cargo publish --workspace
test:
	CARGO_TARGET_DIR=${HOME}/tmp cargo test --lib -- --nocapture

doc:
	cargo doc --no-deps --open

expand:
	CARGO_TARGET_DIR=${HOME}/tmp cargo expand --example rhai > a

clean:
	rm -f a.out
