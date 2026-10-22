## 1. Specification
- [x] 1.1 Define explicit reload and installation/update triggers; remove automatic reload requirements and the superseded proposal.

## 2. Implementation
- [x] 2.1 Remove executable watch and the obsolete CLI flag.
- [x] 2.2 Request reload after installing a release in update and installer.

## 3. Verification
- [x] 3.1 Cover replacement without reload and explicit reload with surviving shells.
- [x] 3.2 Adapt successor refusal, update and installer integration coverage.
- [x] 3.3 Complete review and required checks.
- [x] 3.4 Verify installer version checks precede destination creation, and align reload test references and documentation.
- [x] 3.5 Request explicit reload after SSH deploy replaces the binaries, before committing installation.
