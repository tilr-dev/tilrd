# Report transport

## Failure mechanism
The previous report builder serialized every captured test log into one JSON
string and passed it to `Container.with_new_file`. Dagger records that argument in
span attributes. The report payload was therefore copied through JSON, GraphQL
and telemetry in addition to the filesystem transfer. Large reports could finish
exporting yet fail session shutdown while flushing telemetry, returning exit 1.
The wrapper then stopped before printing the summary and discarded export stdout.

## File snapshot
Write the existing report files under a temporary directory in the module's
scratch workdir. Import that directory using `current_module().workdir` with a
relative path, and await `sync()` before leaving the temporary directory context.
This captures an immutable directory snapshot before removing the local source.
Only the temporary path travels through GraphQL, with no log contents in span
attributes. No per-file overlay layers or unpacking container are needed.

The JSON schema, JUnit structure, captured logs, text summary and aggregate status
retain their existing meanings. Failed Dagger exports keep their diagnostic
stdout and stderr visible to the calling job.

## Regression
The transport stand extracts the actual report function and data class into a
small Dagger module. It creates hundreds of large Unicode logs and exports the
result after the report function has returned and removed its temporary source.
It checks complete log contents and every report format. It also rejects log
payloads in Dagger's plain progress output: this deterministically detects the
original telemetry amplification independently of engine speed or memory size.
