# $env:RUST_BACKTRACE=1; cargo run --example stump
default:
	cargo run --example rhai_mod
	# cargo run --example hello

# cargo publish --workspace
test:
	CARGO_TARGET_DIR=${HOME}/tmp cargo test --lib -- --nocapture

doc:
	cargo doc --no-deps --open

expand:
	cargo expand --example stump > a

clean:
	rm -f a.out
