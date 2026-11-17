## ADDED Requirements

### Requirement: Payloads are bytes the broker does not read
The broker SHALL pass `payload`, `content_type` and `content_encoding` of calls, replies, chunks and events between participants unchanged, byte for byte, including empty and non-UTF-8 payloads. Methods of `tilrd` SHALL take and give the protobuf messages of `crates/tilr-bus/proto/tilr/` (`application/x-protobuf`), and a request with `content_type` `application/json` SHALL be answered in the proto3 JSON mapping.

**Verified by**: `bus::payloads_pass_byte_for_byte`, `bus::core_methods_answer_json_to_json`.

#### Scenario: Binary payload
- **WHEN** the owner calls `test.echo` with 1 MiB of random bytes and `content_type` `application/octet-stream`
- **THEN** the reply has the same bytes and the same `content_type`

#### Scenario: JSON for scripts
- **WHEN** the owner calls `pane.list` with payload `{}` and `content_type` `application/json`
- **THEN** the reply's `content_type` is `application/json` and its payload parses as JSON with a `panes` array
