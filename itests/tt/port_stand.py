"""An opt-in listener in the case's own network namespace."""

import socket
from contextlib import contextmanager


@contextmanager
def occupied_port(case_id: str, target: str):
    if not target or case_id != target:
        yield
        return
    # bind/listen finish before the case runs. No child-process readiness race,
    # inherited listener fd, SO_REUSEPORT, or connection to an unrelated holder.
    with socket.socket() as holder:
        holder.bind(("127.0.0.1", 7890))
        holder.listen()
        print(f"[tt] port stand: {case_id} holds 127.0.0.1:7890", flush=True)
        yield
