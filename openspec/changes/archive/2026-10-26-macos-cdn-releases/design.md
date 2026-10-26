# Design

The release script dispatches macOS builds to a native macOS helper, mapping each platform to its Apple Cargo target. The helper builds both executables, signs them with Developer ID, hardened runtime and a timestamp, verifies their signatures, and submits a ZIP containing those exact binaries to notarytool. Only an Accepted response permits the normal flat CDN archive and checksum to be written. The helper copies accepted binaries into dist/<platform>, which also feeds GitHub packaging.

The CI build job uses the cdn environment. Apple credentials are imported into a temporary keychain before macOS builds and the keychain is removed even on failure. Both macOS platforms always participate in CDN publication; Linux arm64 remains controlled by its existing variable. Upload-only consumes the completed artifacts without Apple credentials or rebuilding.

Integration tests stub external build/sign/notary commands, inspect archived signed bytes and checksum validation, exercise failures before publication, and run the native updater against the layout the release script actually produces. Real Apple service acceptance is required in the release workflow.
