## 1. Local reports
- [x] 1.1 Untrack both reports and ignore their directory; verify `git ls-files openspec/coverage` is empty and `git check-ignore` matches both reports.
- [x] 1.2 Update README, dt, config and the four named active changes to describe local generation; inspect their diff.
- [x] 1.3 Run `./itests/dt coverage`; verify both local reports exist and exit status is 0.
- [x] 1.4 Run `openspec validate --all --strict`; verify all changes and specs validate.
