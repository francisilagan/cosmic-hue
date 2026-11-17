import os
import tempfile
import time
import unittest
from pathlib import Path

import support  # noqa: F401
from hue_backend import wallpaper


def entry(path):
    return '(\n    output: "all",\n    source: Path("%s"),\n    filter_by_theme: true,\n)\n' % path


class CosmicWallpaper(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.cfg = self.root / "cfg"
        self.cfg.mkdir()

    def image(self, name, age=0.0):
        p = self.root / name
        p.write_bytes(b"x")
        t = time.time() - age
        os.utime(p, (t, t))
        return str(p)

    def test_same_on_all_reads_the_all_entry(self):
        (self.cfg / "same-on-all").write_text("true")
        (self.cfg / "all").write_text(entry("/a/b.jpg"))
        self.assertEqual(wallpaper.cosmic_wallpaper(self.cfg), "/a/b.jpg")

    def test_missing_same_on_all_means_all(self):
        (self.cfg / "all").write_text(entry("/a/b.jpg"))
        self.assertEqual(wallpaper.cosmic_wallpaper(self.cfg), "/a/b.jpg")

    def test_per_display_uses_the_first_output(self):
        (self.cfg / "same-on-all").write_text("false")
        (self.cfg / "backgrounds").write_text('["DP-1", "HDMI-A-1"]')
        (self.cfg / "all").write_text(entry("/all.jpg"))
        (self.cfg / "output.DP-1").write_text(entry("/dp1.jpg"))
        self.assertEqual(wallpaper.cosmic_wallpaper(self.cfg), "/dp1.jpg")

    def test_per_display_without_its_entry_falls_back_to_all(self):
        (self.cfg / "same-on-all").write_text("false")
        (self.cfg / "backgrounds").write_text('["DP-1"]')
        (self.cfg / "all").write_text(entry("/all.jpg"))
        self.assertEqual(wallpaper.cosmic_wallpaper(self.cfg), "/all.jpg")

    def test_a_colour_source_has_no_image(self):
        (self.cfg / "all").write_text('(\n    output: "all",\n    source: Color(Single((0.1, 0.2, 0.3))),\n)\n')
        self.assertIsNone(wallpaper.cosmic_wallpaper(self.cfg))

    def test_escaped_quotes_in_the_path_are_unescaped(self):
        (self.cfg / "all").write_text('(source: Path("/x/a \\"b\\".png"))')
        self.assertEqual(wallpaper.cosmic_wallpaper(self.cfg), '/x/a "b".png')

    def test_a_slideshow_folder_resolves_to_its_newest_image(self):
        folder = self.root / "slides"
        folder.mkdir()
        for name, age in (("old.jpg", 100), ("new.png", 1), ("notes.txt", 0)):
            p = folder / name
            p.write_bytes(b"x")
            t = time.time() - age
            os.utime(p, (t, t))
        (self.cfg / "all").write_text(entry(str(folder)))
        self.assertEqual(wallpaper.cosmic_wallpaper(self.cfg), str(folder / "new.png"))

    def test_no_config_has_no_wallpaper(self):
        self.assertIsNone(wallpaper.cosmic_wallpaper(self.cfg))

    def test_the_follower_uses_cosmic_when_no_link_is_given(self):
        img = self.image("w.jpg")
        (self.cfg / "all").write_text(entry(img))
        ws = wallpaper.WallpaperSync(snapshot=lambda: None, submit=lambda c: None, emit=lambda e: None,
                                     extract=lambda p: [], load=lambda: {}, save=lambda d: None)
        orig = wallpaper._cosmic_bg_dir
        wallpaper._cosmic_bg_dir = lambda: self.cfg
        try:
            self.assertEqual(ws._signature()[0], os.path.realpath(img))
        finally:
            wallpaper._cosmic_bg_dir = orig


if __name__ == "__main__":
    unittest.main()
