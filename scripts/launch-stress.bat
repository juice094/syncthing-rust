@echo off
cd /D "C:\Users\22414\dev\third_party\syncthing-rust"
set RUST_BACKTRACE=full
"%~dp0..\target\release\stress_test.exe" --duration 72h --report stress-test-report.csv --data-dir stress-test-data --pid-file stress-test.pid --inject-interval 5m --fault-interval 30m > stress-72h.log 2> stress-72h.err
