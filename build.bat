@echo off
cd /d "%~dp0"
cargo build --release --bin justwrite || exit /b 1
if not exist dist mkdir dist
copy /y target\release\justwrite.exe dist\justwrite.exe >nul || exit /b 1
echo Built dist\justwrite.exe
