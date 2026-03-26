# $env:RUST_BACKTRACE=1; cargo run --example stump
default:
	CARGO_TARGET_DIR=${HOME}/tmp cargo run --example hello

# cargo publish --workspace
test:
	CARGO_TARGET_DIR=${HOME}/tmp cargo test --lib -- --nocapture

doc:
	cargo doc --no-deps --open

expand:
	cargo expand --example stump > a

clean:
	rm -f a.out
