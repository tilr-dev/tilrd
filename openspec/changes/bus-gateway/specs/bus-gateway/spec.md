## ADDED Requirements

### Requirement: The gateway is a built-in instance
A module with `builtin = "gateway"` SHALL run the server's own executable as `tilrd gateway` with the module's and the instance's `args`; it SHALL be supervised like any instance and, after the server reloads into another executable, SHALL be reloaded so it runs the new one.

**Verified by**: `gateway::the_gateway_runs_as_a_built_in_instance`.

#### Scenario: Started from bus.toml
- **WHEN** `bus.toml` has `[modules.gateway] builtin = "gateway"` and instance `http` with `args = ["--listen", "127.0.0.1:0"]`
- **THEN** `tilr bus ls` lists `http` `up`, and its process's command line is the server's executable with `gateway`

### Requirement: The gateway listens on loopback only
`tilrd gateway --listen ADDR:PORT` (default `127.0.0.1:8787`) SHALL accept only loopback addresses; any other address SHALL make it exit with `the gateway listens on loopback only`. A request whose `Host` is not `localhost`, `127.0.0.1` or `[::1]` (with or without port) SHALL be answered 421.

**Verified by**: `gateway::the_gateway_listens_on_loopback_only`.

#### Scenario: Wildcard address
- **WHEN** the instance's args say `--listen 0.0.0.0:8787`
- **THEN** the instance is not `up` and the server log has the gateway's refusal

#### Scenario: Rebound name
- **WHEN** a request to the gateway carries `Host: evil.example`
- **THEN** it is answered 421 and no bus call is made

### Requirement: Browser origins are refused unless allowed
A request with an `Origin` header SHALL be answered 403 unless that origin is given with `--allow-origin` (exact match); requests without `Origin` SHALL pass this check.

**Verified by**: `gateway::browser_origins_are_refused_unless_allowed`.

#### Scenario: Unlisted page
- **WHEN** a request with a valid token carries `Origin: https://example.com`
- **THEN** it is answered 403 and no bus call is made

### Requirement: Every HTTP call carries a token
A request without `Authorization: Bearer <token>` SHALL be answered 401 with `WWW-Authenticate: Bearer` and MUST NOT reach the bus; with one, the call SHALL be made with that token as its credential, and the result SHALL depend only on that token's rights, not on the gateway instance's.

**Verified by**: `gateway::every_http_call_carries_a_token`.

#### Scenario: No token
- **WHEN** `curl -X POST localhost:<port>/call/test.echo -d hi` runs
- **THEN** the status is 401 and the test runtime received no call

#### Scenario: Token with the right
- **WHEN** the same request carries a token minted with `tilr bus token --call test.echo`
- **THEN** the status is 200 and the body is `hi`

#### Scenario: Token without the right
- **WHEN** the request goes to `/call/test.whoami` with that token
- **THEN** the status is 403 and the body's `error.code` is `denied`

### Requirement: Routes and methods
`POST /call/<method>` SHALL call the method routed by method, `POST /call/<instance>/<method>` SHALL call it on that instance; another HTTP method on these paths SHALL be 405, another path 404, an invalid name 400.

**Verified by**: `gateway::routes_and_methods`.

#### Scenario: Targeted call
- **WHEN** a request with a valid token posts to `/call/t-2/test.whoami`
- **THEN** the body is `t-2`

#### Scenario: GET
- **WHEN** `GET /call/test.echo` is sent with a valid token
- **THEN** the status is 405

### Requirement: Bodies and content types pass through
The request body SHALL be the call's payload unchanged and its `Content-Type` the call's `content_type`; the reply's payload SHALL be the response body unchanged with the reply's `content_type` as `Content-Type` (`application/octet-stream` when empty); the response SHALL carry the call's trace id in `X-Tilr-Trace`, and `X-Tilr-Timeout-Ms` SHALL set the call's `timeout_ms`.

**Verified by**: `gateway::bodies_and_content_types_pass_through`.

#### Scenario: Binary body
- **WHEN** a 1 MiB random body with `Content-Type: application/x-test` is posted to `test.echo`
- **THEN** the response body is the same bytes with `Content-Type: application/x-test` and an `X-Tilr-Trace` header

#### Scenario: Short deadline
- **WHEN** `test.sleep` (5 s) is called with `X-Tilr-Timeout-Ms: 300`
- **THEN** the status is 504 after about 300 ms

### Requirement: Bus errors become HTTP statuses
A bus error SHALL be answered with JSON `{"error":{"code","message"}}` and status 403 for `denied`, 404 `not_found`, 503 `unavailable`, 429 `busy` (with `Retry-After`), 504 `timeout`, 413 `too_large`, 400 `bad_request`, 501 `not_supported`, 500 otherwise.

**Verified by**: `gateway::bus_errors_become_http_statuses`.

#### Scenario: Unknown method
- **WHEN** a request with a token that may call everything posts to `/call/nobody.here`
- **THEN** the status is 404 and `error.code` is `not_found`

### Requirement: The gateway bounds what it accepts
A body over 16 MiB minus 64 KiB SHALL be answered 413 without being read whole, headers over 16 KiB 431, and requests beyond 256 in flight 503 without a bus call.

**Verified by**: `gateway::the_gateway_bounds_what_it_accepts`.

#### Scenario: Oversized body
- **WHEN** a request declares `Content-Length` of 17 MiB
- **THEN** the status is 413 and no bus call is made

### Requirement: The gateway does not log secrets
The gateway SHALL log one line per request with method, path, status, duration and trace id, and MUST NOT log the `Authorization` header or bodies.

**Verified by**: `gateway::the_gateway_does_not_log_secrets`.

#### Scenario: Token not in the log
- **WHEN** a request with a token is served
- **THEN** the server log has the request's line and no line contains the token
