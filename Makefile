doc:
	cargo doc

debug:
	cargo build

release:
	cargo build --releases

all: debug

clean:
	cargo clean

test:
	cargo test

bench:
	cargo bench

.PHONY: debug release clean
