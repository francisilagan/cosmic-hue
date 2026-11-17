# Hue Lights for COSMIC

Hue Lights is a panel applet for the [COSMIC](https://system76.com/cosmic) desktop. It controls a Philips Hue system from a popup in the panel.

The applet is written in Rust with `libcosmic`, the official COSMIC toolkit. A small Python backend talks to the bridge. The backend uses only the Python standard library. It is the same backend as in [omarchy-hue](https://github.com/francisilagan/omarchy-hue). The protocol between the applet and the backend is in [docs/PROTOCOL.md](docs/PROTOCOL.md).

## Features

- Rooms and zones: turn a room on or off, and set its brightness.
- Lights: open a room to turn each light on or off and to set its brightness.
- Scenes: open a room and click a scene to recall it. The active scene is highlighted.
- Colour and white: click the colour button of a room or a light. Pick a colour preset, or set the hue and saturation. On the White tab, pick a white preset or set the colour temperature from cool to warm.
- All lights: the switch at the top turns every light on or off.
- Effects: in the colour control of a light, the Effects tab starts an effect (for example candle, fire, or prism). It also starts a 30-minute sunrise or sunset.
- Routines: open a room and click a routine (smart scene) to start or stop it.
- Automations tab: turn the automations of the bridge on or off. Stop an entertainment area that is streaming.
- Devices tab: see the accessories and the bridge, with battery, firmware update state, and sensor readings. Turn sensors such as motion sensing on or off.
- Wallpaper sync: the backend sets the colour of your lights from the current COSMIC wallpaper.
- Live updates: changes from the Hue app or from switches show in the applet.

## Requirements

- COSMIC 1.0 or newer.
- `python3` 3.9 or newer. No pip packages are necessary.
- A Hue bridge (v2, CLIP v2 API) on your local network.
- To build: Rust and `just` (`sudo pacman -S rust just` on Arch).
- Optional, for wallpaper sync only: ImageMagick (`magick`). For video wallpapers, also `ffmpeg`.

## Install

Build and install for your user (no sudo):

```
just install
```

This puts the applet in `~/.local/bin`, the desktop entry in `~/.local/share/applications`, and the backend in `~/.local/share/cosmic-hue/backend`.

Then add the applet to the panel:

1. Open **Settings > Desktop > Panel > Configure panel applets**.
2. Click **Add applet**.
3. Select **Hue Lights**.

To install for all users, run `sudo just prefix=/usr install`.

To remove the applet, run `just uninstall`.

## Pair with the bridge

Click the bulb in the panel. If no bridge is paired, the popup shows the pairing view:

1. **Find my bridge**: the backend searches your local network with mDNS and the neighbour table. Only if it finds no bridge there, it asks the discovery service of Philips. For details, read [docs/PRIVACY.md](docs/PRIVACY.md).
2. Or type the address of the bridge.
3. Click **Pair**. Then press the round link button on the bridge within about 40 seconds.

**Use my openhue login** copies the bridge and the key from the `openhue` CLI config (`~/.config/openhue/config.yaml`).

## Use the applet

| Click on the panel bulb | Action |
|---|---|
| Left | Open the popup. |
| Right | Turn every light on or off. |
| Middle | Refresh. |

## Wallpaper sync

The backend reads the wallpaper from the COSMIC background config (`~/.config/cosmic/com.system76.CosmicBackground/v1/`). With one wallpaper on all displays, it uses that wallpaper. With a wallpaper for each display, it uses the first display. For a slideshow folder, it uses the newest image in the folder. A colour or gradient background has no image, so sync does nothing.

When the wallpaper changes, the backend reads a palette with ImageMagick and sets up to six colours on the colour lights. The backend never changes the brightness. Turn on **Also switch lights on** to include lights that are off.

## Files

| Path | Contents |
|---|---|
| `~/.config/cosmic-hue/bridge.json` (0600) | Bridge host, bridge id, application key, pinned certificate fingerprint |
| `~/.config/cosmic-hue/settings.json` | Wallpaper sync settings |
| `~/.local/state/cosmic-hue/hue.log` (0600) | Rotating diagnostic log |

## Development

```
just test
```

This runs the Python backend tests and the Rust tests.

`python3 backend snapshot` prints a snapshot from your own bridge. To run the applet against a backend in another folder, set `COSMIC_HUE_BACKEND`.

## Not done yet

These features of omarchy-hue are not in the COSMIC applet yet:

- Scene and zone editing, renaming, and the power-up behaviour of a light.
- Translations.

The backend already supports all of them. Only the applet views are missing.

## License

MIT. See [LICENSE](LICENSE).
