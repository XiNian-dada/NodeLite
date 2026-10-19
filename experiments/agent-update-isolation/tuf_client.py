"""Lab-only fixed-scope client; reference TUF trust chain plus application release floor."""

import json

from tuf.ngclient import Updater
from tuf.ngclient.config import UpdaterConfig
from tuf.ngclient.urllib3_fetcher import Urllib3Fetcher

from lab_io import LIMIT, atomic_json, sync_dir
from network_deadline import network_deadline
from tuf_repo import TARGET


class LabPolicyError(ValueError):
    pass


def new_updater(directory, base, bootstrap):
    return Updater(str(directory / "metadata"), base + "/metadata/",
                   str(directory / "downloads"), base + "/targets/",
                   fetcher=Urllib3Fetcher(socket_timeout=1, chunk_size=4096),
                   config=UpdaterConfig(max_root_rotations=8, timestamp_max_length=16384),
                   bootstrap=bootstrap)


def release_info(updater):
    info = updater.get_targetinfo(TARGET)
    if info is None:
        raise LabPolicyError("fixed target not published")
    custom = info.unrecognized_fields.get("custom", {})
    expected = {"product": "nodelite-lab", "component": "agent", "channel": "stable",
                "target": "x86_64-unknown-linux-musl"}
    if set(custom) != set(expected) | {"release_sequence"} \
            or any(custom[key] != value for key, value in expected.items()) \
            or type(custom["release_sequence"]) is not int \
            or not 0 < custom["release_sequence"] < 2**53 or not 0 < info.length <= LIMIT:
        raise LabPolicyError("fixed release scope rejected")
    return info, custom["release_sequence"]


class Client:
    def __init__(self, directory, base, bootstrap):
        self.directory, self.base, self.bootstrap = directory, base, bootstrap
        (directory / "metadata").mkdir(parents=True, exist_ok=True)
        (directory / "downloads").mkdir(exist_ok=True)

    def download(self):
        updater = new_updater(self.directory, self.base, self.bootstrap)
        with network_deadline():
            updater.refresh()
            info, sequence = release_info(updater)
        policy = self.directory / "release-high.json"
        previous = json.loads(policy.read_text()) if policy.exists() else {"sequence": 0}
        if sequence < previous["sequence"] or (sequence == previous["sequence"] and info.hashes != previous.get("hashes")):
            raise LabPolicyError("application release downgrade rejected")
        with network_deadline():
            destination = updater.download_target(info)
        atomic_json(policy, {"sequence": sequence, "hashes": info.hashes})
        sync_dir(self.directory / "metadata")
        return destination
