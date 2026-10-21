# Recover interrupted shared Ghostty builds

An interrupted shared build can leave its directory lock and checkout behind, preventing later invocations from building. Record the owner PID and recover abandoned state automatically. Replace incomplete installation directories before publishing a completed prefix.
