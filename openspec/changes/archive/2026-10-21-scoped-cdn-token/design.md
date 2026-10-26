# Design

Use `aws s3 cp` with `--endpoint-url https://<account>.r2.cloudflarestorage.com` and `--region auto`. Pass the source path, bucket/key, content type and cache control explicitly. The runner provides AWS CLI; the job checks that it exists before uploading. Operators provide S3 credentials through `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`.

The access key ID is the existing account token ID; the secret is its token value hashed with SHA-256. Derive these outside the workflow and store them as environment secrets. No token value or derived secret is committed or logged. Remove the former administrator API token from this environment once the scoped S3 credentials pass the acceptance probe.

Keep archive, checksum and latest publication order per platform. Publish the installer only after every requested platform succeeds. A failed AWS command stops the script. Preserve content types and immutable/no-cache/installer cache policies.

Integration tests run the real shell script against a stub AWS executable. The stub verifies credentials, records argument boundaries and can fail a selected object without network access. Tests check endpoint, region, metadata, platform order and failure propagation. A separate live probe checks object PUT/DELETE and denied ListBuckets using the scoped credentials.
