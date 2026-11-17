## MODIFIED Requirements

### Requirement: Calls are denied unless allowed
A call SHALL be refused with `denied` (`<from> may not call <method>: <reason>`) before routing unless the token that authorizes it (the call's `credential`, else the caller's own token) allows it as the `bus-acl` spec says; `bus.*` methods follow the same rule. The principal `owner` SHALL hold every right.

**Verified by**: `bus::calls_are_denied_unless_allowed`, `bus_acl::an_instance_may_call_what_its_rights_list`.

#### Scenario: Not listed
- **WHEN** `t-1` has `[instances.t-1.rights] call = ["test.echo"]` and calls `test.whoami` and `bus.instances`
- **THEN** both are answered `denied`

#### Scenario: Owner
- **WHEN** the owner calls `bus.instances`
- **THEN** it receives the list
