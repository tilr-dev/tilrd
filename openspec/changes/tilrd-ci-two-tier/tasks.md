## 1. Implementation
- [x] 1.1 Specify the two tiers, the tool cache and the nightly run.
- [x] 1.2 Add the nextest profile `quick` without the slow tests.
- [x] 1.3 Split CI into the quick and the full tier under the same job names; drop the push to `main`; add the nightly and manual runs and the itests job.
- [x] 1.4 Cache the build tools around `scripts/install-tools.sh`.
- [x] 1.5 Extend the static CI tests and document the tiers.
