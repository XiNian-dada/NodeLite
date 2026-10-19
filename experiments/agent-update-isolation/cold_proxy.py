#!/usr/bin/env python3
"""Loopback-only lab frame gate; never logs frames or exports credentials."""

import asyncio
import json
from pathlib import Path

from websockets.asyncio.client import connect
from websockets.asyncio.server import serve

from lab_io import atomic_json
from setup_guest import guard

GATE = Path("/var/lib/nlup-lab/cold/gate.json")


async def transfer(source, destination, direction):
    async for frame in source:
        if isinstance(frame, str) and GATE.exists():
            gate = json.loads(GATE.read_text())
            message = json.loads(frame)
            expected = "refresh_token_response" if direction == "down" else "refresh_token_ack"
            if gate["type"] == expected and message.get("type") == expected \
                    and (direction == "down" or message.get("persisted") is True):
                atomic_json(GATE, {**gate, "reached": True})
                while GATE.exists():
                    await asyncio.sleep(0.05)
        await destination.send(frame)


async def relay(agent):
    try:
        async with connect("ws://127.0.0.1:18088" + agent.request.path) as server:
            tasks = [asyncio.create_task(transfer(agent, server, "up")),
                     asyncio.create_task(transfer(server, agent, "down"))]
            try:
                await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            finally:
                for task in tasks:
                    task.cancel()
                await asyncio.gather(*tasks, return_exceptions=True)
    except Exception:
        # Expected disconnects must not leak frame contents in exception diagnostics.
        await agent.close()


async def main():
    guard()
    async with serve(relay, "127.0.0.1", 18089):
        await asyncio.Future()


if __name__ == "__main__":
    asyncio.run(main())
