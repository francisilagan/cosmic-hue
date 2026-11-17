import os
import stat
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import support  # noqa: F401
from hue_backend import settings


class Settings(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        env = mock.patch.dict(os.environ, {"XDG_CONFIG_HOME": str(Path(self.tmp.name) / "cfg")})
        env.start()
        self.addCleanup(env.stop)

    def test_the_file_lives_in_the_cosmic_hue_config_dir(self):
        self.assertEqual(settings.path(), Path(self.tmp.name) / "cfg" / "cosmic-hue" / "settings.json")

    def test_save_then_load_round_trips_privately(self):
        settings.save({"wallpaper": {"enabled": True}})
        self.assertEqual(settings.load(), {"wallpaper": {"enabled": True}})
        self.assertEqual(stat.S_IMODE(settings.path().stat().st_mode), 0o600)
        self.assertFalse(settings.path().with_name("settings.json.tmp").exists())

    def test_nothing_saved_loads_empty_and_writes_nothing(self):
        self.assertEqual(settings.load(), {})
        self.assertFalse(settings.path().exists())

    def test_a_broken_file_loads_empty(self):
        settings.path().parent.mkdir(parents=True)
        settings.path().write_text("{not json")
        self.assertEqual(settings.load(), {})


if __name__ == "__main__":
    unittest.main()
