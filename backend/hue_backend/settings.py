"""User settings (currently the wallpaper sync options), stored as JSON."""
from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Dict

from . import paths


def path() -> Path:
    """Where the settings file lives."""
    return paths.config_dir() / "settings.json"


def load() -> Dict[str, Any]:
    """The saved settings, or an empty dict when there are none or they cannot be read."""
    try:
        data = json.loads(path().read_text())
        if isinstance(data, dict):
            return data
    except (OSError, ValueError):
        pass
    return {}


def save(data: Dict[str, Any]) -> None:
    """Atomically write the settings file (mode 0600)."""
    paths.write_private(path(), json.dumps(data, indent=2))
