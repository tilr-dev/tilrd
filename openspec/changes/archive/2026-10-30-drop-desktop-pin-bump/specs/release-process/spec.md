## REMOVED Requirements

### Requirement: Stable releases propose the desktop server pin
**Reason**: The desktop application keeps its installed tilrd and tilr on the latest release with `tilrd update`; the bundled server is only an offline starting point and nothing pins its version.
**Migration**: None. A new tilrd release reaches desktop users through the application's update on launch or the Update tilrd button, without a desktop release.
