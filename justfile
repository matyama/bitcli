set positional-arguments := true

alias b := build
alias c := clean
alias d := doc
alias t := test

target := justfile_directory() / "target"

default:
    @just --list

@build profile="dev":
    cargo build --profile {{ profile }} --workspace --all-features --all-targets

lint:
    cargo fmt -q --all --check
    cargo clippy -q --all-targets --all-features -- -D warnings
    cargo doc -q --no-deps --all-features --document-private-items
    cargo outdated -q -R --exit-code=1
    cargo hack -q --feature-powerset check
    cargo deny --log-level=error check -s

@test profile="dev":
    cargo llvm-cov --profile {{ profile }} --workspace --all-features --all-targets

@doc *FLAGS:
    cargo doc -q --workspace --no-deps --all-features --document-private-items {{ FLAGS }}

@clean:
    rm -rf {{ target }}
