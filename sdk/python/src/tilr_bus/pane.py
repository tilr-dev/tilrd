"""Typed helpers for the core's pane methods and events (`tilr.pane.v1`, design T2): a
script creates a pane, types into it and reads its screen without encoding protobuf
by hand."""

from __future__ import annotations

import builtins
import time
from collections.abc import Awaitable, Callable
from typing import TYPE_CHECKING, Any

from tilr_bus.proto import pane

if TYPE_CHECKING:
    from tilr_bus._bus import Bus, ChunkStream, Event, Subscription

#: The window clients open `pane.output` with (frames with images).
PANE_OUTPUT_WINDOW = 4 << 20

#: The events of a pane and their payloads.
EVENTS: dict[str, Any] = {
    "created": pane.PaneInfo,
    "changed": pane.PaneInfo,
    "bell": pane.PaneActivity,
    "command_finished": pane.PaneActivity,
    "copied": pane.Copied,
    "exited": pane.PaneExited,
}


class PaneOutput:
    """`pane.output` as decoded `PaneOutput` frames; `end` is the pane's `PaneExited`
    once it ended. Leaving the loop early cancels the stream."""

    def __init__(self, chunks: ChunkStream) -> None:
        self.chunks = chunks
        self.end: Any = None

    def __aiter__(self) -> PaneOutput:
        return self

    async def __anext__(self) -> Any:
        try:
            data = await self.chunks.__anext__()
        except StopAsyncIteration:
            if self.chunks.end:
                self.end = pane.PaneExited.FromString(self.chunks.end)
            raise
        return pane.PaneOutput.FromString(data)

    def cancel(self) -> None:
        self.chunks.cancel()

    async def __aenter__(self) -> PaneOutput:
        return self

    async def __aexit__(self, *exc: object) -> None:
        self.cancel()


class Panes:
    """The core's pane methods on one connection (`bus.pane`)."""

    def __init__(self, bus: Bus) -> None:
        self.bus = bus

    async def _call(self, method: str, request: Any, reply_type: Any, **options: Any) -> Any:
        reply = await self.bus.call(method, request.SerializeToString(), **options)
        return reply_type.FromString(reply.payload)

    async def create(
        self,
        *,
        cols: int = 0,
        rows: int = 0,
        command: list[str] | None = None,
        cwd: str = "",
        profile: str = "",
        **options: Any,
    ) -> Any:
        """Create a pane (of a profile); answered with its `PaneInfo` once its program runs."""
        request = pane.CreateRequest(
            cols=cols, rows=rows, command=command or [], cwd=cwd, profile=profile
        )
        return await self._call("pane.create", request, pane.PaneInfo, **options)

    async def list(self, **options: Any) -> list[Any]:
        reply = await self._call("pane.list", pane.ListRequest(), pane.ListReply, **options)
        return list(reply.panes)

    async def close(self, pane_id: str, **options: Any) -> int:
        """Close a pane; answered with its exit code once its program ended."""
        request = pane.CloseRequest(pane_id=pane_id)
        reply = await self._call("pane.close", request, pane.CloseReply, **options)
        return reply.exit_code

    async def input(
        self,
        pane_id: str,
        *,
        text: str | None = None,
        key: int | None = None,
        mods: int = 0,
        paste: str | None = None,
        raw: bytes | None = None,
        **options: Any,
    ) -> Any:
        """Send one input (text typed as is, a named key, a paste or raw bytes);
        answered once it was applied."""
        given = [x is not None for x in (text, key, paste, raw)]
        if sum(given) != 1:
            raise ValueError("pass exactly one of text, key, paste, raw")
        if text is not None:
            request = pane.InputRequest(pane_id=pane_id, key=pane.KeyInput(text=text))
        elif key is not None:
            request = pane.InputRequest(
                pane_id=pane_id, key=pane.KeyInput(named=pane.NamedKey.ValueType(key), mods=mods)
            )
        elif paste is not None:
            request = pane.InputRequest(pane_id=pane_id, paste=pane.Paste(text=paste))
        else:
            request = pane.InputRequest(pane_id=pane_id, raw=pane.RawInput(data=raw))
        return await self._call("pane.input", request, pane.InputAck, **options)

    async def type_line(self, pane_id: str, text: str, **options: Any) -> None:
        """Type `text` and press Enter, as two inputs applied in order."""
        await self.input(pane_id, raw=text.encode(), **options)
        await self.input(pane_id, key=pane.NAMED_KEY_ENTER, **options)

    async def resize(self, pane_id: str, cols: int, rows: int, **options: Any) -> Any:
        request = pane.ResizeRequest(pane_id=pane_id, cols=cols, rows=rows)
        return await self._call("pane.resize", request, pane.InputAck, **options)

    async def read(
        self,
        pane_id: str,
        *,
        since_seq: int | None = None,
        wait_ms: int | None = None,
        history_lines: int = 0,
        **options: Any,
    ) -> Any:
        """The screen as text; with `since_seq` it waits for a change after it."""
        request = pane.ReadRequest(pane_id=pane_id, history_lines=history_lines)
        if since_seq is not None:
            request.since_seq = since_seq
        if wait_ms is not None:
            request.wait_ms = wait_ms
        return await self._call("pane.read", request, pane.ReadReply, **options)

    async def wait_for_line(
        self, pane_id: str, match: Callable[[str], bool], timeout: float = 10.0
    ) -> str:
        """Read the screen until a line satisfies `match` (each read waits for a change);
        returns that line, or fails when the program exits first or `timeout` passes."""
        deadline = time.monotonic() + timeout
        since_seq: int | None = None
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                raise TimeoutError(f"no matching line in pane {pane_id} within {timeout} s")
            screen = await self.read(
                pane_id,
                since_seq=since_seq,
                wait_ms=None if since_seq is None else int(left * 1000),
                timeout=left + 1.0,
            )
            for line in screen.lines:
                if match(line):
                    return line
            if not screen.alive:
                raise RuntimeError(f"pane {pane_id} exited before a matching line")
            since_seq = screen.seq

    async def profiles(self, **options: Any) -> builtins.list[Any]:
        reply = await self._call(
            "pane.profiles", pane.ProfilesRequest(), pane.ProfilesReply, **options
        )
        return list(reply.profiles)

    def output(
        self, pane_id: str, *, cell_width_px: int = 0, cell_height_px: int = 0, **options: Any
    ) -> PaneOutput:
        """The pane's screen as a stream of `PaneOutput` (frames, images)."""
        request = pane.OutputRequest(
            pane_id=pane_id, cell_width_px=cell_width_px, cell_height_px=cell_height_px
        )
        options.setdefault("window", PANE_OUTPUT_WINDOW)
        return PaneOutput(self.bus.stream("pane.output", request.SerializeToString(), **options))

    async def on(
        self, event: str, handler: Callable[[Any, Event], Awaitable[None] | None]
    ) -> Subscription:
        """Subscribe to `pane.<event>` with its payload decoded."""
        message_type = EVENTS[event]
        return await self.bus.on(
            f"pane.{event}",
            lambda received: handler(message_type.FromString(received.payload), received),
        )
