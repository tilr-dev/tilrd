"""The Python library against a real tilrd (temporary directories, the TCP port and the
Unix socket), against the Deno library (cross-language) and, where a case needs a
broker that a real server cannot be made to act as (one that misbehaves, or holds a
reply back), against a fake broker. Use cases: USE_CASES.md, "The bus SDKs"."""

from __future__ import annotations

import asyncio
import base64
import os
import sys

import pytest

from tilr_bus import Bus, BusError, Context, Event, Reply, StreamWriter, codes
from tilr_bus._bus import SUBSCRIPTION_QUEUE
from tilr_bus.proto import bus as wire
from tilr_bus.proto import pane

from .harness import (
    Peer,
    Runtime,
    Scope,
    Tilrd,
    UnixListener,
    WsListener,
    runtime_on_fd,
    scenario,
    temp_dir,
)


async def owner(server: Tilrd, scope: Scope, setup=None) -> Bus:
    """The owner on the server's Unix socket."""
    bus = Bus(url=server.unix_url, client_name="py-test")
    if setup is not None:
        setup(bus)
    await bus.connect()
    scope.callback(bus.close)
    return bus


async def raises(awaitable, code: str) -> BusError:
    with pytest.raises(BusError) as caught:
        await awaitable
    assert caught.value.code == code, str(caught.value)
    return caught.value


async def fake_socket(scope: Scope, setup=None, **ack) -> tuple[Bus, Peer]:
    """A Bus on an instance socket the fake broker holds."""
    path = f"{temp_dir(scope)}/p.sock"
    listener = await UnixListener.start(scope, path)
    bus = Bus(socket=(path, "t"))
    if setup is not None:
        setup(bus)
    connecting = asyncio.ensure_future(bus.connect())
    peer = await listener.accept()
    await peer.greet(**ack)
    await connecting
    scope.callback(bus.close)
    return bus, peer


async def flood_state(bus: Bus, method: str) -> dict[str, int | str]:
    """`flood_state` of a runtime: once its flood waits for credit, ended, or got more
    than its window ahead of the reader's credit (bounded: a flood that does none of
    these fails the call, not the test's deadline)."""
    state, sent, credited, ahead, window = (await bus.call(method, timeout=10)).text().split()
    return {
        "state": state,
        "sent": int(sent),
        "credited": int(credited),
        "ahead": int(ahead),
        "window": int(window),
    }


@scenario(60)
async def test_connects_over_fd_socket_and_websocket(scope: Scope) -> None:
    # fd: descriptor 3 is a stream socket the broker holds; the identity is the broker's.
    peer, fd_runtime = await runtime_on_fd(scope, {"TILR_INSTANCE": "py-fd"})
    hello = await peer.greet(identity="instance:py-fd")
    assert hello.client_name == "py-fd"
    assert any(spec.method == "test.whoami" for spec in hello.provides)
    assert await fd_runtime.ready() == "instance:py-fd"
    await peer.send(wire.Envelope(call=wire.Call(id=1, method="test.whoami")))
    answer = await peer.recv()
    assert answer.reply.payload == b"instance:py-fd"

    # socket: the instance socket and its one-time token in Hello.
    path = f"{temp_dir(scope)}/py-sock.sock"
    listener = await UnixListener.start(scope, path)
    sock_runtime = await Runtime.spawn(
        scope, {"TILR_BUS_SOCK": path, "TILR_BUS_TOKEN": "00ff", "TILR_INSTANCE": "py-sock"}
    )
    sock_peer = await listener.accept()
    sock_hello = await sock_peer.greet(identity="instance:py-sock")
    assert sock_hello.instance_token == "00ff"
    assert await sock_runtime.ready() == "instance:py-sock"
    await sock_peer.send(wire.Envelope(call=wire.Call(id=1, method="test.whoami")))
    assert (await sock_peer.recv()).reply.payload == b"instance:py-sock"

    # WebSocket with a credential: sent in Hello as bytes.
    ws = await WsListener.start(scope)
    connecting = asyncio.ensure_future(
        Bus.connect(url=ws.url, credential="AQL6", client_name="py-ws")
    )
    ws_peer, _ = await ws.accept()
    ws_hello = await ws_peer.greet(identity="token:py-ws")
    assert ws_hello.credential == bytes([1, 2, 250])
    ws_bus = await connecting
    scope.callback(ws_bus.close)
    assert ws_bus.identity == "token:py-ws"
    assert ws_bus.credential == bytes([1, 2, 250])

    # A real tilrd: the owner on the Unix socket (`TILR_BUS_URL=unix:...`) and on the TCP
    # port with the access token.
    server = await Tilrd.start(scope)
    runtime = await Runtime.start(
        scope, {"TILR_BUS_URL": server.unix_url, "TEST_CLIENT_NAME": "py-unix"}
    )
    assert await runtime.ready() == "owner"
    tcp = await Bus.connect(url=server.url, access_token=server.token(), client_name="py-tcp")
    scope.callback(tcp.close)
    assert tcp.identity == "owner"
    assert (await tcp.call("test.whoami")).text() == "py-unix"
    await raises(Bus.connect(url=server.url, access_token="not-the-token"), codes.DENIED)

    # Explicit options beat the environment; in it, TILR_BUS_FD beats TILR_BUS_SOCK beats
    # TILR_BUS_URL.
    every = {
        "TILR_BUS_FD": "3",
        "TILR_BUS_SOCK": "/s.sock",
        "TILR_BUS_TOKEN": "t",
        "TILR_BUS_URL": "ws://127.0.0.1:1/",
    }
    probe = Bus()
    assert probe._transport({"environ": every})[0] == "fd"
    assert probe._transport({"environ": {**every, "TILR_BUS_FD": ""}})[0] == "socket"
    assert probe._transport({"environ": {"TILR_BUS_URL": "ws://127.0.0.1:1/"}})[0] == "ws"
    assert probe._transport({"url": "ws://127.0.0.1:2/", "environ": every})[0] == "ws"
    assert probe._transport({"socket": ("/x.sock", "t"), "environ": every})[0] == "socket"

    # A connect that fails leaves the Bus to connect again.
    retry = Bus(environ={}, client_name="py-retry")
    await raises(retry.connect(), codes.UNAVAILABLE)
    retry_dir = temp_dir(scope)
    await raises(retry.connect(socket=(f"{retry_dir}/none.sock", "t")), codes.UNAVAILABLE)
    retry_listener = await UnixListener.start(scope, f"{retry_dir}/r.sock")
    reconnecting = asyncio.ensure_future(retry.connect(socket=(f"{retry_dir}/r.sock", "t")))
    await (await retry_listener.accept()).greet(identity="instance:py-retry")
    assert (await reconnecting).identity == "instance:py-retry"
    scope.callback(retry.close)

    # Nothing to connect to: unavailable, naming the variables.
    error = await raises(Bus.connect(environ={}), codes.UNAVAILABLE)
    for name in ("TILR_BUS_FD", "TILR_BUS_SOCK", "TILR_BUS_TOKEN", "TILR_BUS_URL"):
        assert name in error.message
    assert "TILR_BUS_CREDENTIAL" in error.message


@scenario(60)
async def test_instances_and_a_token_participant_connect_over_fd_socket_and_websocket(
    scope: Scope,
) -> None:
    command = f'["{sys.executable}", "{os.path.dirname(__file__)}/runtime.py"]'

    # Not autostarted: bus.start answers once the instance said Hello, so the calls
    # below need no wait for it to come up.
    def instance(name: str, connect: str) -> str:
        return (
            f'[instances.{name}]\nmodule = "py"\nconnect = "{connect}"\nautostart = false\n'
            f'[instances.{name}.rights]\nprovide = ["test.*"]\n'
        )

    server = await Tilrd.start(
        scope,
        {
            "bus.toml": f"[modules.py]\ncommand = {command}\n"
            + instance("py-fd", "fd")
            + instance("py-sock", "socket")
        },
    )
    bus = await owner(server, scope)
    for name in ("py-fd", "py-sock"):
        await bus.call("bus.start", wire.StartRequest(instance=name).SerializeToString())
    request = wire.TokenRequest(name="py-ws", provide=["test.*"])
    reply = await bus.call("bus.token", request.SerializeToString())
    credential = wire.TokenReply.FromString(reply.payload).credential
    text = base64.urlsafe_b64encode(credential).decode().rstrip("=")
    await Runtime.start(scope, {"TILR_BUS_URL": server.url, "TILR_BUS_CREDENTIAL": text})
    for target in ("instance:py-fd", "instance:py-sock", "token:py-ws"):
        assert (await bus.call("test.whoami", target=target)).text() == target


@scenario(30)
async def test_bytes_pass_unchanged(scope: Scope) -> None:
    server = await Tilrd.start(scope)
    await Runtime.start(scope, {"TILR_BUS_URL": server.unix_url})
    bus = await owner(server, scope)
    reply = await bus.call("test.echo", b"\x00\xff\xfe", content_type="application/octet-stream")
    assert reply.payload == b"\x00\xff\xfe"
    assert reply.content_type == "application/octet-stream"
    # Large enough that this library and the broker compress it: still the same bytes.
    large = bytes(i % 7 for i in range(300_000))
    assert (await bus.call("test.echo", large)).payload == large
    # A 64-bit field keeps every bit: the broker's bus.ping echoes a nonce past 2^53.
    nonce = 2**64 - 3
    pong = await bus.call("bus.ping", wire.PingRequest(nonce=nonce).SerializeToString())
    assert wire.PingReply.FromString(pong.payload).nonce == nonce


@scenario(30)
async def test_errors_keep_their_codes(scope: Scope) -> None:
    server = await Tilrd.start(scope)
    await Runtime.start(scope, {"TILR_BUS_URL": server.unix_url})
    bus = await owner(server, scope)
    failed = await raises(bus.call("test.fail"), codes.BAD_REQUEST)
    assert str(failed) == "bad_request: x"
    crashed = await raises(bus.call("test.crash"), codes.HANDLER_ERROR)
    assert str(crashed) == "handler_error: RuntimeError: a plain failure"
    assert "Traceback" not in crashed.message
    # The broker's errors, with its code and message: a method nobody provides or knows.
    nobody = await raises(bus.call("nobody.here"), codes.NOT_FOUND)
    assert "nobody.here" in nobody.message
    missing = await raises(bus.call("server.nothing"), codes.NOT_FOUND)
    assert "server.nothing" in missing.message

    # A call for a method this participant does not provide: not_supported.
    _client, peer = await fake_socket(scope)
    await peer.send(wire.Envelope(call=wire.Call(id=5, method="test.unknown")))
    answer = await peer.recv()
    assert answer.reply.id == 5
    assert answer.reply.error.code == codes.NOT_SUPPORTED


@scenario(30)
async def test_nested_and_crossing_calls_complete(scope: Scope) -> None:
    server = await Tilrd.start(scope)
    await Runtime.start(scope, {"TILR_BUS_URL": server.unix_url})
    served_meanwhile = asyncio.Event()
    pings = 0

    def setup(bus: Bus) -> None:
        @bus.provide("pt.ping")
        def ping(payload: bytes, ctx: Context) -> str:
            nonlocal pings
            pings += 1
            served_meanwhile.set()
            return "pong"

        # Waits for another call to be served first, then calls the runtime, which calls
        # back into this participant.
        @bus.provide("pt.chain")
        async def chain(payload: bytes, ctx: Context) -> Reply:
            await served_meanwhile.wait()
            return await ctx.call("test.nested", "0 pt.ping")

    await owner(server, scope, setup)
    caller = await owner(server, scope)
    chain = asyncio.ensure_future(caller.call("pt.chain"))
    assert (await caller.call("pt.ping")).text() == "pong"
    assert (await chain).text() == "pong"
    assert pings == 2


@scenario(60)
async def test_python_calls_deno_and_back(scope: Scope) -> None:
    # py-1 handles a call by calling d-1 (the Deno runtime), which calls py-1 again;
    # py-1 serves another call meanwhile.
    server = await Tilrd.start(scope)
    await Runtime.start(
        scope, {"TILR_BUS_URL": server.unix_url, "TEST_NAMESPACE": "d"}, language="deno"
    )
    served_meanwhile = asyncio.Event()
    served: list[str] = []

    def setup(bus: Bus) -> None:
        @bus.provide("py.ping")
        def ping(payload: bytes, ctx: Context) -> str:
            served.append(ctx.from_)
            served_meanwhile.set()
            return "pong from python"

        @bus.provide("py.chain")
        async def chain(payload: bytes, ctx: Context) -> Reply:
            await served_meanwhile.wait()
            return await ctx.call("d.nested", "0 py.ping")

    await owner(server, scope, setup)
    caller = await owner(server, scope)
    chain = asyncio.ensure_future(caller.call("py.chain", timeout=20))
    assert (await caller.call("py.ping")).text() == "pong from python"
    assert (await chain).text() == "pong from python"
    assert len(served) == 2
    # And the other way: Deno answers Python's call with Python's own reply.
    assert (await caller.call("d.echo", b"\x00\xff")).payload == b"\x00\xff"
    assert (await caller.call("d.nested", "0 py.ping")).text() == "pong from python"


@scenario(60)
async def test_events_are_emitted_and_received(scope: Scope) -> None:
    server = await Tilrd.start(scope)
    deno = await Runtime.start(scope, {"TILR_BUS_URL": server.unix_url}, language="deno")
    bus = await owner(server, scope)
    # Python hears Deno.
    events: list[Event] = []
    arrived = asyncio.Event()

    def heard(event: Event) -> None:
        events.append(event)
        arrived.set()

    subscription = await bus.on("test.*", heard)
    await bus.call("test.emit", "test.ping 1")
    await arrived.wait()
    assert events[0].subject == "test.ping"
    assert events[0].payload == b"1"
    assert events[0].from_ == await deno.ready()  # the runtime's identity, set by the broker
    # Unsubscribed: a later event of that pattern is not delivered, one of another is.
    subscription.unsubscribe()
    arrived.clear()
    await bus.on("mark.*", heard)
    await bus.call("test.emit", "test.ping 2")
    await bus.call("test.emit", "mark.done x")
    await arrived.wait()
    assert [event.subject for event in events] == ["test.ping", "mark.done"]
    # Without a handler, the subscription is read with `async for`.
    async with await bus.on("loop.*") as loop:
        await bus.call("test.emit", "loop.one 1")
        async for event in loop:
            assert (event.subject, event.payload) == ("loop.one", b"1")
            break
    # This library emits too, and Deno hears it.
    await bus.call("test.subscribe", "pt.*")
    await bus.emit("pt.hello", "there")
    assert "event pt.hello owner there" in (await bus.call("test.records", "1")).text()

    # A denied subscription fails with denied.
    client, peer = await fake_socket(scope, identity="instance:py-1")
    subscribing = asyncio.ensure_future(client.on("pane.*", heard))
    subscribe = (await peer.recv()).subscribe
    assert subscribe.pattern == "pane.*"
    error = wire.Error(code="denied", message="instance:py-1 may not subscribe to pane.*")
    await peer.send(wire.Envelope(reply=wire.Reply(id=subscribe.id, error=error)))
    await raises(subscribing, codes.DENIED)


async def read_a_flood(bus: Bus, namespace: str, total: int, window: int) -> None:
    """Read `<namespace>.flood` of `total` bytes with `window`: one chunk, then the
    reader stops and the provider waits for credit, no more than the window beyond what
    it was credited; then the rest, in order, never more than the window ahead."""
    # Bounded: a provider that stops sending (a gate that ignores credit) fails the
    # stream `timeout` at its deadline rather than holding the test to its own.
    stream = bus.stream(f"{namespace}.flood", str(total), window=window, timeout=20)
    async with stream as chunks:
        first = await chunks.__anext__()
        received = len(first)
        ordered = first.count(b"x") == len(first)
        waiting = await flood_state(bus, f"{namespace}.flood_state")
        assert waiting["state"] == "waiting", waiting
        assert waiting["window"] == window
        assert int(waiting["sent"]) - int(waiting["credited"]) <= window, waiting
        async for chunk in chunks:
            ordered = ordered and chunk.count(b"x") == len(chunk)
            received += len(chunk)
    assert received == total
    assert ordered
    ended = await flood_state(bus, f"{namespace}.flood_state")
    assert ended["state"] == "ended", ended
    assert ended["sent"] == total
    assert int(ended["ahead"]) <= window, f"the provider got {ended['ahead']} bytes ahead"


@scenario(60)
async def test_streams_are_read_and_served_with_credit(scope: Scope) -> None:
    server = await Tilrd.start(scope)
    await Runtime.start(scope, {"TILR_BUS_URL": server.unix_url}, language="deno")
    bus = await owner(server, scope)
    # The Deno runtime's 64 MiB with a 64 KiB window, paced by this reader's credit.
    await read_a_flood(bus, "test", 64 << 20, 64 << 10)

    # Python's own runtime: its flood paced the same way; count, every chunk in order
    # and the End.
    await Runtime.start(
        scope, {"TILR_BUS_URL": server.unix_url, "TEST_NAMESPACE": "py"}, language="python"
    )
    await read_a_flood(bus, "py", 8 << 20, 64 << 10)
    counted = [chunk.decode() async for chunk in bus.stream("py.count", "5")]
    assert counted == ["1", "2", "3", "4", "5"]

    # Leaving `async with` early cancels the stream: the provider sees it.
    async with bus.stream("py.count", "1000000", window=16) as numbers:
        async for chunk in numbers:
            assert chunk == b"1"
            break
    assert "cancelled py.count" in (await bus.call("py.records", "1", timeout=5)).text()

    # This library provides a stream, paced by the reader's credit.
    credits: list[int] = []

    def setup(provider: Bus) -> None:
        @provider.provide("pt.numbers")
        async def numbers(payload: bytes, ctx: Context, out: StreamWriter) -> str:
            for i in range(4):
                await out.write(bytes([i]) * 8)
                credits.append(out.credit)
            return "done"

    await owner(server, scope, setup)
    numbers = bus.stream("pt.numbers", window=16)
    got = [chunk[0] async for chunk in numbers]
    assert got == [0, 1, 2, 3]
    assert numbers.end == b"done"
    assert all(0 <= credit <= 16 for credit in credits), credits


@scenario(60)
async def test_a_script_drives_a_terminal(scope: Scope) -> None:
    server = await Tilrd.start(
        scope, {"profiles/demo.toml": 'description = "demo"\ncommand = ["/bin/sh"]\n'}
    )
    # The script holds only what it needs: the pane calls, pane.exited, panes of profile
    # demo and its own panes.
    request = wire.TokenRequest(
        name="py-script",
        call=["pane.*"],
        subscribe=["pane.exited"],
        pane=["profile:demo", "access:own"],
    )
    minted = await (await owner(server, scope)).call("bus.token", request.SerializeToString())
    credential = wire.TokenReply.FromString(minted.payload).credential
    bus = await Bus.connect(url=server.url, credential=credential, client_name="py-script")
    scope.callback(bus.close)
    assert bus.identity == "token:py-script"
    info = await bus.pane.create(cols=80, rows=24, profile="demo")
    exited: asyncio.Future = asyncio.get_running_loop().create_future()

    def on_exit(event, _received) -> None:
        if event.pane_id == info.pane_id and not exited.done():
            exited.set_result(event)

    await bus.pane.on("exited", on_exit)
    await bus.pane.type_line(info.pane_id, "echo sum $((6*7))")
    line = await bus.pane.wait_for_line(info.pane_id, lambda line: line.strip() == "sum 42")
    assert line.strip() == "sum 42"
    await bus.pane.close(info.pane_id)
    assert (await exited).pane_id == info.pane_id


@scenario(30)
async def test_pane_helpers_speak_the_pane_schema(scope: Scope) -> None:
    bus, peer = await fake_socket(scope, identity="instance:py-1")

    creating = asyncio.ensure_future(bus.pane.create(cols=80, rows=24, profile="demo"))
    create = (await peer.recv()).call
    assert create.method == "pane.create"
    assert pane.CreateRequest.FromString(create.payload).profile == "demo"
    info = pane.PaneInfo(pane_id="p-1", cols=80).SerializeToString()
    await peer.send(wire.Envelope(reply=wire.Reply(id=create.id, payload=info)))
    assert (await creating).pane_id == "p-1"

    ack = pane.InputAck(pane_id="p-1").SerializeToString()
    typing = asyncio.ensure_future(bus.pane.type_line("p-1", "echo hi"))
    first = (await peer.recv()).call
    assert pane.InputRequest.FromString(first.payload).raw.data == b"echo hi"
    await peer.send(wire.Envelope(reply=wire.Reply(id=first.id, payload=ack)))
    second = (await peer.recv()).call
    assert pane.InputRequest.FromString(second.payload).key.named == pane.NAMED_KEY_ENTER
    await peer.send(wire.Envelope(reply=wire.Reply(id=second.id, payload=ack)))
    await typing

    reading = asyncio.ensure_future(bus.pane.read("p-1"))
    read = (await peer.recv()).call
    screen = pane.ReadReply(seq=3, lines=["$ echo hi", "hi"], alive=True)
    await peer.send(wire.Envelope(reply=wire.Reply(id=read.id, payload=screen.SerializeToString())))
    assert list((await reading).lines) == ["$ echo hi", "hi"]

    # Events decoded; the subscription is answered first.
    arrived: asyncio.Future = asyncio.get_running_loop().create_future()
    subscribing = asyncio.ensure_future(
        bus.pane.on("exited", lambda event, _received: arrived.set_result(event))
    )
    subscribe = (await peer.recv()).subscribe
    assert subscribe.pattern == "pane.exited"
    await peer.send(wire.Envelope(reply=wire.Reply(id=subscribe.id)))
    await subscribing
    exited = pane.PaneExited(pane_id="p-1", exit_code=3).SerializeToString()
    event = wire.Event(subject="pane.exited", payload=exited)
    setattr(event, "from", "tilrd")
    await peer.send(wire.Envelope(event=event))
    assert await arrived == pane.PaneExited(pane_id="p-1", exit_code=3)


@scenario(30)
async def test_a_cancel_aborts_the_handler(scope: Scope) -> None:
    server = await Tilrd.start(scope)
    await Runtime.start(scope, {"TILR_BUS_URL": server.unix_url})
    bus = await owner(server, scope)
    await raises(bus.call("test.sleep", "10000", timeout=0.3), codes.TIMEOUT)
    records = await bus.call("test.records", "1", timeout=1)
    assert "aborted test.sleep" in records.text()
    # ctx.call passes the time left and the trace id on.
    left = int((await bus.call("test.nested", "0 test.deadline", timeout=5)).text())
    assert 0 < left <= 5000, f"the nested call had {left} ms"
    assert (await bus.call("test.nested", "0 test.trace", trace_id="trace-7")).text() == "trace-7"
    # This library's handler sees its deadline and is cancelled.
    seen: dict[str, float | bool | None] = {}
    cancelled = asyncio.Event()

    def setup(provider: Bus) -> None:
        @provider.provide("pt.wait")
        async def wait(payload: bytes, ctx: Context) -> None:
            seen["left"] = ctx.time_left()
            try:
                await asyncio.Event().wait()  # never set
            except asyncio.CancelledError:
                seen["flag"] = ctx.cancelled.is_set()
                cancelled.set()
                raise

    await owner(server, scope, setup)
    await raises(bus.call("pt.wait", timeout=0.2), codes.TIMEOUT)
    await cancelled.wait()
    left_s = seen["left"]
    assert isinstance(left_s, float) and 0 < left_s <= 0.2
    assert seen["flag"] is True


@scenario(30)
async def test_a_cancelled_call_gets_no_reply(scope: Scope) -> None:
    # One handler is cancelled as the bus cancels it, one swallows the cancellation and
    # answers anyway: the broker's Cancel means neither answer goes out.
    started = [asyncio.Event(), asyncio.Event()]

    def setup(bus: Bus) -> None:
        @bus.provide("pt.stops", kind="unary")
        async def stops(payload: bytes, ctx: Context) -> str:
            started[0].set()
            await asyncio.Event().wait()  # never set: cancelled
            return "never"

        @bus.provide("pt.ignores", kind="unary")
        async def ignores(payload: bytes, ctx: Context) -> str:
            started[1].set()
            try:
                await asyncio.Event().wait()
            except asyncio.CancelledError:
                pass  # swallowed: it answers after the cancel
            return "late"

        @bus.provide("pt.marker", kind="unary")
        def marker(payload: bytes, ctx: Context) -> str:
            return "marker"

    _bus, peer = await fake_socket(scope, setup)
    await peer.send(wire.Envelope(call=wire.Call(id=7, method="pt.stops")))
    await peer.send(wire.Envelope(call=wire.Call(id=8, method="pt.ignores")))
    await asyncio.gather(*(event.wait() for event in started))
    await peer.send(wire.Envelope(cancel=wire.Cancel(id=7, reason="timeout")))
    await peer.send(wire.Envelope(cancel=wire.Cancel(id=8, reason="timeout")))
    # Handled in order: the marker's reply is the next thing the library sends.
    await peer.send(wire.Envelope(call=wire.Call(id=9, method="pt.marker")))
    answer = await peer.recv()
    assert answer.WhichOneof("msg") == "reply", answer
    assert answer.reply.id == 9, f"a cancelled call was answered: {answer}"
    assert answer.reply.payload == b"marker"


@scenario(30)
async def test_a_subscriber_that_is_behind_drops_events(scope: Scope) -> None:
    bus, peer = await fake_socket(scope)
    got: list[bytes] = []
    first = asyncio.Event()
    release = asyncio.Event()
    every = asyncio.Event()

    async def handler(event: Event) -> None:
        got.append(event.payload)
        if len(got) == 1:
            first.set()
            await release.wait()  # the handler is behind
        if len(got) == 1 + SUBSCRIPTION_QUEUE + 1:
            every.set()

    subscribing = asyncio.ensure_future(bus.on("pt.*", handler))
    subscribe = (await peer.recv()).subscribe
    await peer.send(wire.Envelope(reply=wire.Reply(id=subscribe.id)))
    subscription = await subscribing

    def event(n: int) -> wire.Envelope:
        sent = wire.Event(subject="pt.tick", payload=str(n).encode())
        setattr(sent, "from", "owner")
        return wire.Envelope(event=sent)

    await peer.send(event(0))
    await first.wait()
    # While the handler is busy with the first: 1024 wait, 5 more are dropped.
    for n in range(1, SUBSCRIPTION_QUEUE + 6):
        await peer.send(event(n))
    # Handled in order: once the marker is answered, every event above was taken.
    await peer.send(wire.Envelope(call=wire.Call(id=99, method="pt.marker")))
    assert (await peer.recv()).reply.error.code == codes.NOT_SUPPORTED
    assert subscription.dropped == 5
    # The subscription stays: the queued ones are handled in order, and later ones too.
    release.set()
    await peer.send(event(SUBSCRIPTION_QUEUE + 6))
    await every.wait()
    assert len(got) == 1 + SUBSCRIPTION_QUEUE + 1
    assert got[:3] == [b"0", b"1", b"2"]
    assert got[SUBSCRIPTION_QUEUE] == str(SUBSCRIPTION_QUEUE).encode()
    assert got[-1] == str(SUBSCRIPTION_QUEUE + 6).encode()
    assert subscription.dropped == 5


@scenario(10)
async def test_leaving_async_with_cancels_the_stream(scope: Scope) -> None:
    # One slot in flight: the call after the block goes out only once the stream's slot
    # is free, and the Cancel comes first.
    bus, peer = await fake_socket(scope, max_in_flight=1)
    stream = bus.stream("pt.numbers", window=16)
    async with stream:
        reading = asyncio.ensure_future(stream.__anext__())
        call = (await peer.recv()).call
        assert call.method == "pt.numbers"
        await peer.send(wire.Envelope(chunk=wire.Chunk(id=call.id, payload=b"1")))
        assert await reading == b"1"
    # `stream` is still held here: the cancel is the block's, not the collector's.
    calling = asyncio.ensure_future(bus.call("pt.after"))
    try:
        async with asyncio.timeout(2):
            credit = await peer.recv()
            assert credit.WhichOneof("msg") == "credit", credit
            cancel = await peer.recv()
            assert cancel.WhichOneof("msg") == "cancel", cancel
            assert cancel.cancel.id == call.id
            after = (await peer.recv()).call
    except TimeoutError:
        raise AssertionError("leaving `async with` did not cancel the stream at once") from None
    assert after.method == "pt.after"
    await peer.send(wire.Envelope(reply=wire.Reply(id=after.id, payload=b"ok")))
    assert (await calling).payload == b"ok"
    assert stream is not None


@scenario(60)
async def test_limits_are_respected(scope: Scope) -> None:
    server = await Tilrd.start(scope)
    await Runtime.start(scope, {"TILR_BUS_URL": server.unix_url})
    bus = await owner(server, scope)
    # tilrd answers `busy` past max_in_flight (256) pending calls of a connection: 1000
    # at once all complete only because the library keeps the rest waiting.
    assert bus.hello_ack.max_in_flight == 256
    replies = await asyncio.gather(*(bus.call("test.sleep", "20") for _ in range(1000)))
    assert all(reply.text() == "slept" for reply in replies)
    # Over max_frame: too_large, nothing sent, the connection stays.
    await raises(bus.call("test.echo", bytes(17 << 20)), codes.TOO_LARGE)
    assert (await bus.call("test.echo", "still")).text() == "still"

    # At most max_in_flight calls pending: the fourth waits for a reply.
    client, peer = await fake_socket(scope, max_in_flight=3)
    calls = [asyncio.ensure_future(client.call("test.echo", str(i))) for i in range(5)]
    ids = [(await peer.recv()).call.id for _ in range(3)]
    await client.emit("pt.marker")
    marker = await peer.recv()
    assert marker.WhichOneof("msg") == "event", "a fourth call was sent before a slot was free"
    await peer.send(wire.Envelope(reply=wire.Reply(id=ids[0])))
    fourth = (await peer.recv()).call
    assert fourth.payload == b"3"
    for call_id in (ids[1], ids[2], fourth.id):
        await peer.send(wire.Envelope(reply=wire.Reply(id=call_id)))
    fifth = (await peer.recv()).call
    assert fifth.payload == b"4"
    await peer.send(wire.Envelope(reply=wire.Reply(id=fifth.id)))
    assert len(await asyncio.gather(*calls)) == 5


@scenario(30)
async def test_a_closed_connection_ends_pending_calls(scope: Scope) -> None:
    server = await Tilrd.start(scope)
    await Runtime.start(scope, {"TILR_BUS_URL": server.unix_url})
    bus = await owner(server, scope)
    pending = asyncio.ensure_future(bus.call("test.sleep", "60000"))
    stream = bus.stream("test.count", "1000000", window=4)
    assert await stream.__anext__() == b"1"
    assert (await bus.call("test.echo", "sent")).text() == "sent"  # the sleep is on its way
    await server.kill()
    await raises(pending, codes.UNAVAILABLE)
    # What came before the end is still read, then the stream fails.
    with pytest.raises(BusError) as ended:
        async for _ in stream:
            pass
    assert ended.value.code == codes.UNAVAILABLE
    info = await bus.wait_closed()
    assert info is not None
    assert bus.is_closed
    await raises(bus.call("test.echo"), codes.UNAVAILABLE)

    # The latest CredentialUpdate is the connection's credential.
    ws = await WsListener.start(scope)
    connecting = asyncio.ensure_future(Bus.connect(url=ws.url, credential=b"\x01"))
    peer, connection = await ws.accept()
    await peer.greet(identity="token:py-ws")
    client = await connecting
    scope.callback(client.close)
    await peer.send(wire.Envelope(credential_update=wire.CredentialUpdate(credential=b"\x09\x09")))
    # Taken in order: once the call after it is answered, the update was read.
    await peer.send(wire.Envelope(call=wire.Call(id=1, method="test.unknown")))
    assert (await peer.recv()).reply.error.code == codes.NOT_SUPPORTED
    assert client.credential == b"\x09\x09"
    # A ConnectionError and then the close: both are kept, the close's code too.
    await peer.send(
        wire.Envelope(connection_error=wire.ConnectionError(code="too_slow", message="behind"))
    )
    await connection.close(4001, "too slow")
    closed = await client.wait_closed()
    assert (closed.code, closed.error) == (4001, ("too_slow", "behind")), closed
    await raises(client.call("test.echo"), "too_slow")
