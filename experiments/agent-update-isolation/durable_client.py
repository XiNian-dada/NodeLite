"""Separate experimental TUF cache transaction; no activation or credential access."""

from pathlib import Path
import time

from cache_store import Store
from lab_io import LIMIT, read_file
from network_deadline import network_deadline
from tuf_client import LabPolicyError, new_updater, release_info


class ClockRollback(LabPolicyError):
    pass


class DurableClient:
    def __init__(self, directory, base, bootstrap, checkpoint=lambda _: None):
        self.store, self.base, self.bootstrap = Store(directory, bootstrap), base, bootstrap
        self.checkpoint = checkpoint

    def download(self):
        with self.store.lock():
            state = self.store.load()
            now = time.time()
            if now + 5 < state["time_high"]:
                raise ClockRollback("clock precedes the accepted local time floor")
            directory = self.store.stage(state)
            updater = new_updater(directory, self.base, self.bootstrap)
            try:
                with network_deadline():
                    updater.refresh()
                    info, sequence = release_info(updater)
                if "sha256" not in info.hashes:
                    raise LabPolicyError("lab artifact requires sha256")
                if sequence < state["sequence"] or (sequence == state["sequence"] and info.hashes != state["hashes"]):
                    raise LabPolicyError("seen release cannot be downgraded or equivocated")
            except Exception:
                # The reference client can learn a revoked root before a later role fails.
                self.store.publish_metadata(state, directory)
                raise
            self.checkpoint("metadata_staged")
            state = self.store.publish_metadata(state, directory, sequence=sequence, hashes=info.hashes, now=now)
            self.checkpoint("metadata_committed")
            with network_deadline():
                destination = updater.download_target(info)
            payload = read_file(Path(destination), LIMIT)
            return self.store.publish_artifact(state, payload, sequence, self.checkpoint)
