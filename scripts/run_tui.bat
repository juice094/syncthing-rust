@echo off
mode con: cols=120 lines=25
start "syncthing-rust TUI" "C:\Users\22414\dev\syncthing-rust\target\release\syncthing.exe" tui --config-dir "C:\Users\22414\AppData\Local\syncthing-rust"
