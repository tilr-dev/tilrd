## 1. Specification
- [x] 1.1 Specify macOS builds, signing, notarization and CDN publication.

## 2. Implementation
- [x] 2.1 Add both macOS platforms to the release script and CI, with signing and accepted notarization before packaging.
- [x] 2.2 Document CI credentials and complete the macOS CDN decision in release-platforms.

## 3. Verification
- [x] 3.1 Verify both architectures publish archives, checksums and latest with signed bytes, including failure paths.
- [x] 3.2 Verify native macOS update from the local CDN without renaming another platform's archive.
- [x] 3.3 Run checks and review the final diff.
