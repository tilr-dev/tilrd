# Tasks

## 0. Check

- [x] 0.1 UC-004 on `dev`: fixed by 3ba0e93 in `itests/installer/install.sh` (all cases green per its commit and itests/USE_CASES.md group A)

## 1. Port

- [ ] 1.1 Copy `itests/installer/install.sh` into the `sh` repository; verify with `diff` against `itests/installer/install.sh` (empty)
- [ ] 1.2 Publish it to R2 before the first archive-only release reaches the CDN; verify `curl -fsSL https://tilr.sh | diff - itests/installer/install.sh` is empty
- [ ] 1.3 Run itests group A (`./dt --only scenarios --filter uc_00`); verify UC-001..006 PASS
