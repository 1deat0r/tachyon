$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root

if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
    throw "rustup is required. Install it from https://rustup.rs/"
}

rustup toolchain install 1.98.1 --profile default --component rustfmt --component clippy
rustup override set 1.98.1

cargo fmt --check
cargo check --workspace
cargo test --workspace

Write-Host "Tachyon scaffold validated. Read README_FOR_HERMES.md and begin Milestone 0."
