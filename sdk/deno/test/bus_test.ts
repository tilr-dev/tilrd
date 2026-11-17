// The Deno library against a real tilrd (temporary directories, the TCP port and the
// Unix socket) and, where the server side is not there yet or a case needs a broker
// that misbehaves, against a fake broker. Use cases: USE_CASES.md, "The bus SDKs".

import { assert, assertEquals, assertRejects, assertStringIncludes } from "@std/assert";
import { Bus, BusError, type BusEvent, codes, pane, resolveTransport } from "../mod.ts";
import {
  listenUnix,
  listenWs,
  Runtime,
  runtimeOnFd,
  scenario,
  type Scope,
  text,
  Tilrd,
} from "./harness.ts";

/** The owner on the server's Unix socket. */
async function owner(server: Tilrd, scope: Scope, setup?: (bus: Bus) => void): Promise<Bus> {
  const bus = new Bus({ url: server.unixUrl, clientName: "deno-test" });
  setup?.(bus);
  await bus.connect();
  scope.defer(() => bus.close());
  return bus;
}

async function rejectsWith(promise: Promise<unknown>, code: string): Promise<BusError> {
  const error = await assertRejects(() => promise, BusError);
  assertEquals((error as BusError).code, code, String(error));
  return error as BusError;
}

scenario("connects_over_fd_socket_and_websocket", 60_000, async (scope) => {
  // fd: descriptor 3 is a stream socket the broker holds; the identity is the broker's.
  const fd = runtimeOnFd(scope, { TILR_INSTANCE: "d-fd" });
  const hello = await fd.peer.greet({ identity: "instance:d-fd" });
  assertEquals(hello.clientName, "d-fd");
  assert((hello.provides as { method: string }[]).some((spec) => spec.method === "test.whoami"));
  await fd.stdout.wait(
    "d-fd is not ready",
    (line) => (line === "ready instance:d-fd" ? true : undefined),
  );
  fd.peer.send({ msg: "call", call: { id: 1, method: "test.whoami", from: "owner" } });
  const answer = await fd.peer.recv();
  assertEquals(answer.msg, "reply");
  assertEquals(text(answer.reply.payload), "instance:d-fd");

  // socket: the instance socket and its one-time token in Hello.
  const path = `${Deno.makeTempDirSync({ dir: "/tmp", prefix: "tb" })}/d-sock.sock`;
  scope.defer(() => Deno.removeSync(path.replace(/\/[^/]+$/, ""), { recursive: true }));
  const listener = await listenUnix(scope, path);
  const sock = Runtime.spawn(scope, {
    TILR_BUS_SOCK: path,
    TILR_BUS_TOKEN: "00ff",
    TILR_INSTANCE: "d-sock",
  });
  const peer = await listener.accept();
  const sockHello = await peer.greet({ identity: "instance:d-sock" });
  assertEquals(sockHello.instanceToken, "00ff");
  assertEquals(await sock.ready(), "instance:d-sock");
  peer.send({ msg: "call", call: { id: 1, method: "test.whoami" } });
  assertEquals(text((await peer.recv()).reply.payload), "instance:d-sock");

  // WebSocket with a credential: sent in Hello as bytes.
  const ws = listenWs(scope);
  const credential = new Uint8Array([1, 2, 250]);
  const connecting = Bus.connect({ url: ws.url, credential: "AQL6", clientName: "d-ws" });
  const { peer: wsPeer } = await ws.accept();
  const wsHello = await wsPeer.greet({ identity: "token:d-ws" });
  assertEquals(wsHello.credential, credential);
  const wsBus = await connecting;
  scope.defer(() => wsBus.close());
  assertEquals(wsBus.identity, "token:d-ws");
  assertEquals(wsBus.credential, credential);

  // A real tilrd: the owner on the Unix socket (`TILR_BUS_URL=unix:...`) and on the TCP
  // port with the access token.
  const server = await Tilrd.start(scope);
  const runtime = await Runtime.start(scope, {
    TILR_BUS_URL: server.unixUrl,
    TEST_CLIENT_NAME: "d-unix",
  });
  assertEquals(await runtime.ready(), "owner");
  const tcp = await Bus.connect({
    url: server.url,
    accessToken: server.token(),
    clientName: "d-tcp",
  });
  scope.defer(() => tcp.close());
  assertEquals(tcp.identity, "owner");
  assertEquals((await tcp.call("test.whoami")).text(), "d-unix");
  await rejectsWith(Bus.connect({ url: server.url, accessToken: "not-the-token" }), codes.DENIED);

  // Nothing to connect to: unavailable, naming the variables.
  try {
    resolveTransport({ env: {} });
    throw new Error("resolved a transport from an empty environment");
  } catch (error) {
    assertEquals((error as BusError).code, codes.UNAVAILABLE);
    for (
      const name of [
        "TILR_BUS_FD",
        "TILR_BUS_SOCK",
        "TILR_BUS_TOKEN",
        "TILR_BUS_URL",
        "TILR_BUS_CREDENTIAL",
      ]
    ) {
      assertStringIncludes((error as BusError).message, name);
    }
  }
});

scenario(
  "instances_and_a_token_participant_connect_over_fd_socket_and_websocket",
  60_000,
  async (scope) => {
    const runtime = `["${Deno.execPath()}", "run", "--quiet", "-A", "${
      new URL("./runtime.ts", import.meta.url).pathname
    }"]`;
    const server = await Tilrd.start(scope, {
      "bus.toml":
        `[modules.deno]\ncommand = ${runtime}\n[instances.d-fd]\nmodule = "deno"\nconnect = "fd"\n` +
        `[instances.d-sock]\nmodule = "deno"\nconnect = "socket"\n`,
    });
    const bus = await owner(server, scope);
    const token = await bus.call(
      "bus.token",
      (await import("../mod.ts")).bus.TokenRequest.encode({ name: "d-ws", provide: ["test.*"] }),
    );
    const credential = (await import("../mod.ts")).bus.TokenReply.decode(token.payload).credential!;
    const b64 = btoa(String.fromCharCode(...credential)).replace(/\+/g, "-").replace(/\//g, "_")
      .replace(/=+$/, "");
    await Runtime.start(scope, { TILR_BUS_URL: server.url, TILR_BUS_CREDENTIAL: b64 });
    for (const target of ["instance:d-fd", "instance:d-sock", "token:d-ws"]) {
      assertEquals((await bus.call("test.whoami", "", { target })).text(), target);
    }
  },
  { ignore: true }, // needs the server side of bus-instances (bus.toml) and bus-rights (bus.token)
);

scenario("bytes_pass_unchanged", 30_000, async (scope) => {
  const server = await Tilrd.start(scope);
  await Runtime.start(scope, { TILR_BUS_URL: server.unixUrl });
  const bus = await owner(server, scope);
  const reply = await bus.call("test.echo", new Uint8Array([0x00, 0xff, 0xfe]), {
    contentType: "application/octet-stream",
  });
  assertEquals(reply.payload, new Uint8Array([0x00, 0xff, 0xfe]));
  assertEquals(reply.contentType, "application/octet-stream");
  // Large enough for the broker to compress it on the WebSocket: still the same bytes.
  const large = new Uint8Array(300_000).map((_, i) => i % 7);
  assertEquals((await bus.call("test.echo", large)).payload, large);
});

scenario("errors_keep_their_codes", 30_000, async (scope) => {
  const server = await Tilrd.start(scope);
  await Runtime.start(scope, { TILR_BUS_URL: server.unixUrl });
  const bus = await owner(server, scope);
  const failed = await rejectsWith(bus.call("test.fail"), codes.BAD_REQUEST);
  assertEquals(String(failed), "bad_request: x");
  const crashed = await rejectsWith(bus.call("test.crash"), codes.HANDLER_ERROR);
  assertEquals(String(crashed), "handler_error: Error: a plain failure");
  // The broker's errors, with its code and message.
  const nobody = await assertRejects(() => bus.call("nobody.here"), BusError);
  assertStringIncludes(nobody.message, "nobody.here");
  assert(([codes.NOT_FOUND, codes.UNAVAILABLE] as string[]).includes(nobody.code), String(nobody));
  const missing = await rejectsWith(bus.call("server.nothing"), codes.NOT_FOUND);
  assertStringIncludes(missing.message, "server.nothing");

  // A call for a method this participant does not provide: not_supported.
  const path = `${Deno.makeTempDirSync({ dir: "/tmp", prefix: "tb" })}/p.sock`;
  const listener = await listenUnix(scope, path);
  const connecting = Bus.connect({ socket: { path, token: "t" } });
  const peer = await listener.accept();
  await peer.greet();
  const client = await connecting;
  scope.defer(() => client.close());
  peer.send({ msg: "call", call: { id: 5, method: "test.unknown" } });
  const answer = await peer.recv();
  assertEquals(answer.reply.id, 5);
  assertEquals(answer.reply.error.code, codes.NOT_SUPPORTED);
});

scenario("nested_and_crossing_calls_complete", 30_000, async (scope) => {
  const server = await Tilrd.start(scope);
  await Runtime.start(scope, { TILR_BUS_URL: server.unixUrl });
  let pinged!: () => void;
  const servedMeanwhile = new Promise<void>((resolve) => (pinged = resolve));
  let pings = 0;
  const local = await owner(server, scope, (bus) => {
    bus.provide("dt.ping", () => {
      pings++;
      pinged();
      return "pong";
    });
    // Waits for another call to be served first, then calls the runtime, which calls
    // back into this participant.
    bus.provide("dt.chain", async (_payload, ctx) => {
      await servedMeanwhile;
      return (await ctx.call("test.nested", "0 dt.ping")).payload;
    });
  });
  const caller = await owner(server, scope);
  const chain = caller.call("dt.chain");
  assertEquals((await caller.call("dt.ping")).text(), "pong");
  assertEquals((await chain).text(), "pong");
  assertEquals(pings, 2);
  assert(local.identity === "owner");
});

scenario("events_are_emitted_and_received", 30_000, async (scope) => {
  const server = await Tilrd.start(scope);
  const runtime = await Runtime.start(scope, { TILR_BUS_URL: server.unixUrl });
  const bus = await owner(server, scope);
  const events: BusEvent[] = [];
  let got!: () => void;
  let arrived = new Promise<void>((resolve) => (got = resolve));
  const off = await bus.on("test.*", (event) => {
    events.push(event);
    got();
  });
  await bus.call("test.emit", "test.ping 1");
  await arrived;
  assertEquals(events[0].subject, "test.ping");
  assertEquals(text(events[0].payload), "1");
  assertEquals(events[0].from, await runtime.ready()); // the runtime's identity, set by the broker
  // Unsubscribed: a later event of that pattern is not delivered, one of another is.
  off();
  arrived = new Promise<void>((resolve) => (got = resolve));
  await bus.on("mark.*", (event) => {
    events.push(event);
    got();
  });
  await bus.call("test.emit", "test.ping 2");
  await bus.call("test.emit", "mark.done x");
  await arrived;
  assertEquals(events.map((event) => event.subject), ["test.ping", "mark.done"]);
  // This library emits too.
  await bus.call("test.subscribe", "dt.*");
  await bus.emit("dt.hello", "there");
  assertStringIncludes((await bus.call("test.records", "1")).text(), "event dt.hello owner there");

  // A denied subscription fails with denied.
  const path = `${Deno.makeTempDirSync({ dir: "/tmp", prefix: "tb" })}/p.sock`;
  const listener = await listenUnix(scope, path);
  const connecting = Bus.connect({ socket: { path, token: "t" } });
  const peer = await listener.accept();
  await peer.greet({ identity: "instance:d-1" });
  const client = await connecting;
  scope.defer(() => client.close());
  const subscribing = client.on("pane.*", () => {});
  const subscribe = await peer.recv();
  assertEquals(subscribe.subscribe.pattern, "pane.*");
  peer.send({
    msg: "reply",
    reply: {
      id: subscribe.subscribe.id,
      error: { code: "denied", message: "instance:d-1 may not subscribe to pane.*" },
    },
  });
  await rejectsWith(subscribing, codes.DENIED);
});

scenario("streams_are_read_and_served_with_credit", 120_000, async (scope) => {
  const server = await Tilrd.start(scope);
  await Runtime.start(scope, { TILR_BUS_URL: server.unixUrl });
  const bus = await owner(server, scope);
  // A slow reader of 64 MiB with a 64 KiB window: everything, in order, and the
  // provider's memory stays bounded because it waits for credit.
  const total = 64 << 20;
  let received = 0;
  let ordered = true;
  for await (const chunk of bus.stream("test.flood", String(total), { window: 64 << 10 })) {
    if (chunk.some((byte) => byte !== 0x78)) ordered = false;
    received += chunk.length;
    await new Promise((resolve) => setTimeout(resolve, 10)); // the slow consumer
  }
  assertEquals(received, total);
  assert(ordered);
  // A Deno process holds some 45 MB before it serves anything: what is measured is
  // what serving the stream added, which buffering it would make 64 MiB or more.
  const [peak, before] = (await bus.call("test.memory")).text().split(" ").map(Number);
  assert(
    before > 0 && peak - before < 64 << 20,
    `serving grew the provider from ${before} to ${peak} bytes`,
  );

  // Count: every chunk in order and the End.
  const counted: string[] = [];
  for await (const chunk of bus.stream("test.count", "5")) counted.push(text(chunk));
  assertEquals(counted, ["1", "2", "3", "4", "5"]);

  // Leaving the loop early cancels the stream: the provider sees it.
  for await (const chunk of bus.stream("test.count", "1000000", { window: 16 })) {
    assertEquals(text(chunk), "1");
    break;
  }
  assertStringIncludes((await bus.call("test.records", "1")).text(), "cancelled test.count");

  // This library provides a stream, paced by the reader's credit.
  const writes: number[] = [];
  const provider = await owner(server, scope, (b) => {
    b.provide("dt.numbers", async (_payload, _ctx, out) => {
      for (let i = 0; i < 4; i++) {
        await out.write(new Uint8Array(8).fill(i));
        writes.push(out.credit);
      }
      return "done";
    });
  });
  const numbers = bus.stream("dt.numbers", "", { window: 16 });
  const got: number[] = [];
  for await (const chunk of numbers) got.push(chunk[0]);
  assertEquals(got, [0, 1, 2, 3]);
  assertEquals(text(numbers.end!), "done");
  assert(writes.every((credit) => credit >= 0 && credit <= 16), `credit seen ${writes}`);
  assert(provider.identity === "owner");
});

scenario("a_script_drives_a_terminal", 60_000, async (scope) => {
  const server = await Tilrd.start(scope, {
    "profiles/demo.toml": 'description = "demo"\ncommand = ["/bin/sh"]\n',
  });
  const bus = await owner(server, scope);
  const info = await bus.pane.create({ cols: 80, rows: 24, profile: "demo" });
  let exited!: (event: pane.PaneExited) => void;
  const exitedEvent = new Promise<pane.PaneExited>((resolve) => (exited = resolve));
  await bus.pane.on("exited", (event) => {
    if (event.paneId === info.paneId) exited(event);
  });
  await bus.pane.typeLine(info.paneId!, "echo sum $((6*7))");
  assertEquals(
    await bus.pane.waitForLine(info.paneId!, (line) => line.trim() === "sum 42"),
    "sum 42",
  );
  await bus.pane.close(info.paneId!);
  assertEquals((await exitedEvent).paneId, info.paneId);
}, { ignore: true }); // needs the server side of bus-core-panes and pane-profiles

scenario("pane_helpers_speak_the_pane_schema", 30_000, async (scope) => {
  const path = `${Deno.makeTempDirSync({ dir: "/tmp", prefix: "tb" })}/p.sock`;
  const listener = await listenUnix(scope, path);
  const connecting = Bus.connect({ socket: { path, token: "t" } });
  const peer = await listener.accept();
  await peer.greet({ identity: "instance:d-1" });
  const bus = await connecting;
  scope.defer(() => bus.close());

  const creating = bus.pane.create({ cols: 80, rows: 24, profile: "demo" });
  const create = (await peer.recv()).call;
  assertEquals(create.method, "pane.create");
  assertEquals(pane.CreateRequest.decode(create.payload).profile, "demo");
  peer.send({
    msg: "reply",
    reply: { id: create.id, payload: pane.PaneInfo.encode({ paneId: "p-1", cols: 80 }) },
  });
  assertEquals((await creating).paneId, "p-1");

  const typing = bus.pane.typeLine("p-1", "echo hi");
  const first = (await peer.recv()).call;
  assertEquals(text(pane.InputRequest.decode(first.payload).raw!.data!), "echo hi");
  peer.send({
    msg: "reply",
    reply: { id: first.id, payload: pane.InputAck.encode({ paneId: "p-1" }) },
  });
  const second = (await peer.recv()).call;
  assertEquals(pane.InputRequest.decode(second.payload).key!.named, pane.NamedKey.NAMED_KEY_ENTER);
  peer.send({
    msg: "reply",
    reply: { id: second.id, payload: pane.InputAck.encode({ paneId: "p-1" }) },
  });
  await typing;

  const reading = bus.pane.read({ paneId: "p-1" });
  const read = (await peer.recv()).call;
  peer.send({
    msg: "reply",
    reply: {
      id: read.id,
      payload: pane.ReadReply.encode({ seq: 3, lines: ["$ echo hi", "hi"], alive: true }),
    },
  });
  assertEquals((await reading).lines, ["$ echo hi", "hi"]);

  // Events decoded; the subscription is answered first.
  const events: pane.PaneExited[] = [];
  let got!: () => void;
  const arrived = new Promise<void>((resolve) => (got = resolve));
  const subscribing = bus.pane.on("exited", (event) => {
    events.push(event);
    got();
  });
  const subscribe = (await peer.recv()).subscribe;
  assertEquals(subscribe.pattern, "pane.exited");
  peer.send({ msg: "reply", reply: { id: subscribe.id } });
  await subscribing;
  peer.send({
    msg: "event",
    event: {
      subject: "pane.exited",
      payload: pane.PaneExited.encode({ paneId: "p-1", exitCode: 3 }),
      from: "tilrd",
    },
  });
  await arrived;
  assertEquals(events[0], { paneId: "p-1", exitCode: 3 });
});

scenario("a_cancel_aborts_the_handler", 30_000, async (scope) => {
  const server = await Tilrd.start(scope);
  await Runtime.start(scope, { TILR_BUS_URL: server.unixUrl });
  const bus = await owner(server, scope);
  await rejectsWith(bus.call("test.sleep", "10000", { timeoutMs: 300 }), codes.TIMEOUT);
  const records = await bus.call("test.records", "1", { timeoutMs: 1000 });
  assertStringIncludes(records.text(), "aborted test.sleep");
  // ctx.call passes the time left and the trace id on.
  const left = Number(
    (await bus.call("test.nested", "0 test.deadline", { timeoutMs: 5000 })).text(),
  );
  assert(left > 0 && left <= 5000, `the nested call had ${left} ms`);
  assertEquals(
    (await bus.call("test.nested", "0 test.trace", { traceId: "trace-7" })).text(),
    "trace-7",
  );
  // This library's handler sees its deadline and its signal.
  let seen: { left?: number; aborted?: boolean } = {};
  let done!: () => void;
  const aborted = new Promise<void>((resolve) => (done = resolve));
  await owner(server, scope, (b) => {
    b.provide("dt.wait", (_payload, ctx) => {
      seen = { left: ctx.timeLeftMs() };
      ctx.signal.addEventListener("abort", () => {
        seen.aborted = true;
        done();
      });
      return new Promise<never>(() => {});
    });
  });
  await rejectsWith(bus.call("dt.wait", "", { timeoutMs: 200 }), codes.TIMEOUT);
  await aborted;
  assert(seen.left! > 0 && seen.left! <= 200);
});

scenario("limits_are_respected", 60_000, async (scope) => {
  const server = await Tilrd.start(scope);
  await Runtime.start(scope, { TILR_BUS_URL: server.unixUrl });
  const bus = await owner(server, scope);
  const replies = await Promise.all(
    Array.from({ length: 1000 }, () => bus.call("test.sleep", "20")),
  );
  assert(replies.every((reply) => reply.text() === "slept"));
  // Over max_frame: too_large, nothing sent, the connection stays.
  await rejectsWith(bus.call("test.echo", new Uint8Array(17 << 20)), codes.TOO_LARGE);
  assertEquals((await bus.call("test.echo", "still")).text(), "still");

  // At most max_in_flight calls pending: the fourth waits for a reply.
  const path = `${Deno.makeTempDirSync({ dir: "/tmp", prefix: "tb" })}/p.sock`;
  const listener = await listenUnix(scope, path);
  const connecting = Bus.connect({ socket: { path, token: "t" } });
  const peer = await listener.accept();
  await peer.greet({ maxInFlight: 3 });
  const client = await connecting;
  scope.defer(() => client.close());
  const calls = Array.from({ length: 5 }, (_, i) => client.call("test.echo", String(i)));
  const all = Promise.all(calls);
  all.catch(() => {}); // a failure below is the test's
  const ids: number[] = [];
  for (let i = 0; i < 3; i++) ids.push((await peer.recv()).call.id);
  await client.emit("dt.marker");
  assertEquals((await peer.recv()).msg, "event", "a fourth call was sent before a slot was free");
  peer.send({ msg: "reply", reply: { id: ids[0], payload: new Uint8Array() } });
  const fourth = (await peer.recv()).call;
  assertEquals(text(fourth.payload), "3");
  for (const id of [ids[1], ids[2], fourth.id]) {
    peer.send({ msg: "reply", reply: { id, payload: new Uint8Array() } });
  }
  const fifth = (await peer.recv()).call;
  assertEquals(text(fifth.payload), "4");
  peer.send({ msg: "reply", reply: { id: fifth.id, payload: new Uint8Array() } });
  assertEquals((await all).length, 5);
});

scenario("a_closed_connection_ends_pending_calls", 30_000, async (scope) => {
  const server = await Tilrd.start(scope);
  await Runtime.start(scope, { TILR_BUS_URL: server.unixUrl });
  const bus = await owner(server, scope);
  const pending = bus.call("test.sleep", "60000");
  const stream = bus.stream("test.count", "1000000", { window: 4 });
  assertEquals(text((await stream.next()).value!), "1");
  assertEquals((await bus.call("test.echo", "sent")).text(), "sent"); // the sleep is on its way
  const failed = rejectsWith(pending, codes.UNAVAILABLE);
  await server.kill();
  await failed;
  // What came before the end is still read, then the stream fails.
  const ended = await (async () => {
    try {
      for await (const _ of stream) { /* chunks sent before the kill */ }
    } catch (error) {
      return error as BusError;
    }
  })();
  assertEquals(ended?.code, codes.UNAVAILABLE);
  const info = await bus.closed;
  assert(info !== undefined);
  assert(bus.isClosed);
  await rejectsWith(bus.call("test.echo"), codes.UNAVAILABLE);

  // The latest CredentialUpdate is the connection's credential.
  const ws = listenWs(scope);
  const connecting = Bus.connect({ url: ws.url, credential: new Uint8Array([1]) });
  const { peer } = await ws.accept();
  await peer.greet({ identity: "token:d-ws" });
  const client = await connecting;
  scope.defer(() => client.close());
  peer.send({ msg: "credentialUpdate", credentialUpdate: { credential: new Uint8Array([9, 9]) } });
  // Taken in order: once the call after it is answered, the update was read.
  peer.send({ msg: "call", call: { id: 1, method: "test.unknown" } });
  assertEquals((await peer.recv()).reply.error.code, codes.NOT_SUPPORTED);
  assertEquals(client.credential, new Uint8Array([9, 9]));
  peer.close();
  await client.closed;
});
