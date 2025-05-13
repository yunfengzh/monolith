# $env:RUST_BACKTRACE=1; cargo run --example stump
default:
	cargo run --example hello

# $env:RUST_TEST_NOCAPTURE=1; cargo test --lib
test:
	cargo test --lib

doc:
	cargo doc --no-deps --open

expand:
	cargo expand --example stump > a

clean:
	rm -f a.out
