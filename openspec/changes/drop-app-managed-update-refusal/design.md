## Approach

Remove the marker guard before release discovery. The update path keeps its checksum verification, atomic replacement, and reload request. There is no new state to transfer on reload or behavior to change on client reconnect.

Replace the refusal test with a local-feed installation test that checks both installed executables and preservation of the marker file.
