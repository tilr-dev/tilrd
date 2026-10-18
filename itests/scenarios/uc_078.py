"""UC-078: real SSH agent forwarding through the client, as `ssh -A` gives it: a program in
a pane uses the keys of the agent on the user's machine. `ssh-add -l` in the pane lists
the client's keys; `git clone` over ssh from a git server authenticates with them; and
`ssh -A` from the pane to that server forwards the same agent one hop further.

Three containers: "laptop", the user's machine: an ssh-agent holding a fresh ed25519 key
and tilr-driver relaying the server's agent streams to it (`agent` mode socket,
"agent" in Hello.features); the case's own, the server, where the pane runs; and "git":
sshd with that key in root's authorized_keys and a bare repository. The private key
never leaves the laptop.
"""

from tt import T, case

SSH = "ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o BatchMode=yes -o LogLevel=ERROR"


def laptop_agent(t: T, laptop):
    """An ssh-agent on the laptop with one key; its socket and the key's fingerprint."""
    sock = "/tmp/agent.sock"
    laptop.sh(f"rm -f {sock} /tmp/id /tmp/id.pub && ssh-agent -a {sock} </dev/null >/dev/null 2>&1 && "
              "ssh-keygen -q -t ed25519 -N '' -C laptop-key -f /tmp/id && "
              f"SSH_AUTH_SOCK={sock} ssh-add -q /tmp/id", check=True)
    listed = laptop.sh(f"SSH_AUTH_SOCK={sock} ssh-add -l", check=True).out.strip()
    public = laptop.run("cat", "/tmp/id.pub", check=True).out.strip()
    return sock, listed, public


def git_server(t: T, git, public: str):
    """sshd on the git host taking the laptop's key for root; /srv/repo.git with a commit."""
    git.sh("mkdir -p /run/sshd /root/.ssh && chmod 700 /root/.ssh && ssh-keygen -A >/dev/null && "
           f"echo '{public}' >/root/.ssh/authorized_keys && chmod 600 /root/.ssh/authorized_keys && "
           "/usr/sbin/sshd </dev/null >/dev/null 2>&1 && "
           "git init -q --bare /srv/repo.git && git init -q /tmp/work && cd /tmp/work && "
           "echo 'forwarded agent works' >README && git add README && "
           "git -c user.name=tt -c user.email=tt@tt commit -q -m first && "
           "git push -q /srv/repo.git HEAD:refs/heads/main && git -C /srv/repo.git symbolic-ref HEAD refs/heads/main",
           check=True)


def setup(t: T, forward: bool = True):
    laptop, git = t.host("laptop"), t.host("git")
    sock, listed, public = laptop_agent(t, laptop)
    git_server(t, git, public)
    server = t.server(listen="0.0.0.0:0", state=t.root / "state")
    d = laptop.driver("client")
    d.connect(f"ws://{laptop.peer_ip}:{server.port}", token=server.token(),
              features=["ping", "agent"] if forward else ["ping"])
    if forward:
        d.cmd("agent", mode="socket", socket=sock)
    pane = d.shell_pane(120, 30)
    return laptop, git, server, d, pane, listed, public


@case(fixture="server_state", hosts=["laptop", "git"], spec=[
    "ssh-agent-forwarding/real-ssh-and-git-use-the-forwarded-agent",
])
def ssh_add_in_a_pane_lists_the_keys_of_the_clients_agent(t: T):
    """UC-078: ssh-add -l / -L in a pane show exactly the key held by the agent on the client's machine."""
    laptop, git, server, d, pane, listed, public = setup(t)
    out = t.scratch("ssh") / "list"
    t.eq(d.run(pane, f"ssh-add -l >{out}"), 0, "ssh-add -l in the pane succeeds")
    t.eq(out.read_text().strip(), listed, "the pane's agent lists the laptop's key (fingerprint)")
    t.eq(d.run(pane, f"ssh-add -L >{out}"), 0, "ssh-add -L succeeds")
    t.eq(out.read_text().strip(), public, "and its public key")
    t.ok(not list(t.root.glob("**/id")), "no private key on the server's machine")


@case(fixture="server_state", hosts=["laptop", "git"], spec=[
    "ssh-agent-forwarding/real-ssh-and-git-use-the-forwarded-agent",
])
def git_clone_over_ssh_uses_the_forwarded_agent_and_ssh_a_forwards_it_further(t: T):
    """UC-078: git clone root@git:/srv/repo.git in a pane authenticates with the client's key; ssh -A git ssh-add -l too."""
    laptop, git, server, d, pane, listed, public = setup(t)
    dest = t.scratch("clone") / "repo"
    status = d.run(pane, f"GIT_SSH_COMMAND='{SSH}' git clone -q root@git:/srv/repo.git {dest}", timeout=60)
    t.eq(status, 0, "git clone over ssh succeeds")
    t.eq((dest / "README").read_text(), "forwarded agent works\n", "the clone has the repository's file")
    out = t.scratch("ssh") / "hop"
    t.eq(d.run(pane, f"{SSH} -A root@git ssh-add -l >{out}", timeout=30), 0, "ssh -A to the git host succeeds")
    t.eq(out.read_text().strip(), listed, "the agent forwarded one hop further lists the laptop's key")


@case(fixture="server_state", hosts=["laptop", "git"], spec=[
    "ssh-agent-forwarding/with-no-agent-client-the-program-finds-no-agent",
])
def without_the_clients_agent_the_clone_is_refused(t: T):
    """UC-078: a client that does not forward its agent: ssh-add finds no keys, git clone is denied."""
    laptop, git, server, d, pane, listed, public = setup(t, forward=False)
    t.ok(d.run(pane, "ssh-add -l") != 0, "ssh-add -l finds no agent keys")
    dest = t.scratch("clone") / "repo"
    t.ok(d.run(pane, f"GIT_SSH_COMMAND='{SSH}' git clone -q root@git:/srv/repo.git {dest}", timeout=60) != 0,
         "git clone over ssh is denied without the client's agent")
