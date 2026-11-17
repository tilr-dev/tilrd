## ADDED Requirements

### Requirement: The terminal is served on the bus only
The server SHALL serve panes, files, exec, the clipboard, host events and the SSH agent only as methods, events and streams of `tilr.bus.v1`, provided or emitted by the participant `tilrd`; `crates/tilr-proto/proto/tilr.proto` and the package `tilr.v1` MUST NOT exist in the repository, except for the one refusal packet of the `component-bus` spec.

**Verified by**: `bus_terminal::tilrd_provides_the_terminal_methods`, CI step `! test -e crates/tilr-proto` in `.github/workflows/ci.yml`.

#### Scenario: Methods listed
- **WHEN** the owner calls `bus.methods` on a server started with default options
- **THEN** `tilrd` is the provider of `pane.create`, `pane.list`, `pane.close`, `pane.resize`, `pane.input`, `pane.read`, `pane.applied`, `pane.input_finalize`, `pane.profiles`, `pane.output`, `server.info`, `server.reload`, `server.update`, `server.command`, `fs.list`, `fs.stat`, `fs.read`, `fs.write`, `fs.mkdir`, `fs.rename`, `fs.delete`, `fs.watch`, `fs.unwatch`, `exec.run`, `exec.input`, `exec.signal`, `clipboard.put` and `clipboard.get`
