@echo off
cd /d "%~dp0"
cargo run --profile fast --bin justwrite -- %*
