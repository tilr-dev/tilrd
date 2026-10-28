# Event routing

## Decision
Use a pass-through success step for merge_group in the existing spec-change job. Every PR must pass authorization before queue admission; authorization concerns each task and its explicit grant, while CI already checks the combined merge result. Rechecking every PR in a group would require additional API discovery and task credentials without adding a new authorization boundary.

All existing authorization steps run only on pull_request_target. The queue step uses no PR payload, checkout, checker downloads, or task credential. Keep the job name spec-change for the required status context.

The rollout uses the already merged gitpm checker commit and matching digests in downstream repositories; no shared checker script changes are needed. Rulesets are enabled by the main session only after these workflows have landed in every repository.

See [GitHub merge queue documentation](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue) for event and admission semantics.
