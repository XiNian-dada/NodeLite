"""Lab-only fixed-scope client; reference TUF trust chain plus application release floor."""

import json

from tuf.ngclient import Updater
from tuf.ngclient.config import UpdaterConfig

from lab_io import LIMIT, atomic_json, sync_dir
from tuf_repo import TARGET


class LabPolicyError(ValueError):
    pass


class Client:
    def __init__(self, directory, base, bootstrap):
        self.directory, self.base, self.bootstrap = directory, base, bootstrap
        (directory / "metadata").mkdir(parents=True, exist_ok=True)
        (directory / "downloads").mkdir(exist_ok=True)

    def download(self):
        updater = Updater(str(self.directory / "metadata"), self.base + "/metadata/",
                          str(self.directory / "downloads"), self.base + "/targets/",
                          config=UpdaterConfig(max_root_rotations=8, timestamp_max_length=16384),
                          bootstrap=self.bootstrap)
        updater.refresh()
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
        policy = self.directory / "release-high.json"
        previous = json.loads(policy.read_text()) if policy.exists() else {"sequence": 0}
        if custom["release_sequence"] < previous["sequence"] or (
                custom["release_sequence"] == previous["sequence"] and info.hashes != previous.get("hashes")):
            raise LabPolicyError("application release downgrade rejected")
        destination = updater.download_target(info)
        atomic_json(policy, {"sequence": custom["release_sequence"], "hashes": info.hashes})
        sync_dir(self.directory / "metadata")
        return destination
