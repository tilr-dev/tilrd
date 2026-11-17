"""The Python library against a real tilrd (temporary directories, the TCP port and the
Unix socket), against the Deno library (cross-language) and, where the server side is
not there yet or a case needs a broker that misbehaves, against a fake broker. Use
cases: USE_CASES.md, "The bus SDKs"."""

from __future__ import annotations

import asyncio
import base64
import os
import sys

import pytest

from tilr_bus import Bus, BusError, Context, Event, Reply, StreamWriter, codes
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


async def fake_socket(scope: Scope, **ack) -> tuple[Bus, Peer]:
    """A Bus on an instance socket the fake broker holds."""
    path = f"{temp_dir(scope)}/p.sock"
    listener = await UnixListener.start(scope, path)
    connecting = asyncio.ensure_future(Bus.connect(socket=(path, "t")))
    peer = await listener.accept()
    await peer.greet(**ack)
    bus = await connecting
    scope.callback(bus.close)
    return bus, peer


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

    # Nothing to connect to: unavailable, naming the variables.
    error = await raises(Bus.connect(environ={}), codes.UNAVAILABLE)
    for name in ("TILR_BUS_FD", "TILR_BUS_SOCK", "TILR_BUS_TOKEN", "TILR_BUS_URL"):
        assert name in error.message
    assert "TILR_BUS_CREDENTIAL" in error.message


@pytest.mark.skip(
    reason="needs the server side of bus-instances (bus.toml) and bus-rights (bus.token)"
)
@scenario(60)
async def test_instances_and_a_token_participant_connect_over_fd_socket_and_websocket(
    scope: Scope,
) -> None:
    command = f'["{sys.executable}", "{os.path.dirname(__file__)}/runtime.py"]'
    server = await Tilrd.start(
        scope,
        {
            "bus.toml": f'[modules.py]\ncommand = {command}\n[instances.py-fd]\nmodule = "py"\n'
            'connect = "fd"\n[instances.py-sock]\nmodule = "py"\nconnect = "socket"\n'
        },
    )
    bus = await owner(server, scope)
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
    # The broker's errors, with its code and message.
    with pytest.raises(BusError) as nobody:
        await bus.call("nobody.here")
    assert "nobody.here" in nobody.value.message
    assert nobody.value.code in (codes.NOT_FOUND, codes.UNAVAILABLE), str(nobody.value)
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


@scenario(120)
async def test_streams_are_read_and_served_with_credit(scope: Scope) -> None:
    server = await Tilrd.start(scope)
    await Runtime.start(scope, {"TILR_BUS_URL": server.unix_url}, language="deno")
    bus = await owner(server, scope)
    # A slow reader of the Deno runtime's 64 MiB with a 64 KiB window: everything, in
    # order, and the provider's memory stays bounded because it waits for credit.
    total = 64 << 20
    received = 0
    ordered = True
    async for chunk in bus.stream("test.flood", str(total), window=64 << 10):
        ordered = ordered and chunk.count(b"x") == len(chunk)
        received += len(chunk)
        await asyncio.sleep(0.01)  # the slow consumer
    assert received == total
    assert ordered
    # A Deno process holds some 45 MB before it serves anything: what is measured is
    # what serving the stream added, which buffering it would make 64 MiB or more.
    peak, before = map(int, (await bus.call("test.memory")).text().split())
    assert before > 0 and peak - before < 64 << 20, f"the provider grew {before} -> {peak}"

    # Python's own runtime: count, every chunk in order and the End.
    await Runtime.start(
        scope, {"TILR_BUS_URL": server.unix_url, "TEST_NAMESPACE": "py"}, language="python"
    )
    counted = [chunk.decode() async for chunk in bus.stream("py.count", "5")]
    assert counted == ["1", "2", "3", "4", "5"]

    # Leaving the loop early cancels the stream: the provider sees it.
    async for chunk in bus.stream("py.count", "1000000", window=16):
        assert chunk == b"1"
        break
    assert "cancelled py.count" in (await bus.call("py.records", "1")).text()

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


@pytest.mark.skip(reason="needs the server side of bus-core-panes and pane-profiles")
@scenario(60)
async def test_a_script_drives_a_terminal(scope: Scope) -> None:
    server = await Tilrd.start(
        scope, {"profiles/demo.toml": 'description = "demo"\ncommand = ["/bin/sh"]\n'}
    )
    bus = await owner(server, scope)
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


@scenario(60)
async def test_limits_are_respected(scope: Scope) -> None:
    server = await Tilrd.start(scope)
    await Runtime.start(scope, {"TILR_BUS_URL": server.unix_url})
    bus = await owner(server, scope)
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
    peer, _ = await ws.accept()
    await peer.greet(identity="token:py-ws")
    client = await connecting
    scope.callback(client.close)
    await peer.send(wire.Envelope(credential_update=wire.CredentialUpdate(credential=b"\x09\x09")))
    # Taken in order: once the call after it is answered, the update was read.
    await peer.send(wire.Envelope(call=wire.Call(id=1, method="test.unknown")))
    assert (await peer.recv()).reply.error.code == codes.NOT_SUPPORTED
    assert client.credential == b"\x09\x09"
    await peer.close()
    await client.wait_closed()
