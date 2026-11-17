# Build and install the COSMIC Hue applet.
#
#   just install           # to ~/.local (no sudo)
#   sudo just prefix=/usr install

prefix := env_var('HOME') / '.local'
appid := 'io.github.francisilagan.CosmicHue'
bin := prefix / 'bin' / 'cosmic-applet-hue'
share := prefix / 'share'

default: build

build:
    cargo build --release --manifest-path applet/Cargo.toml

test:
    python3 -m unittest discover -s tests
    cargo test --manifest-path applet/Cargo.toml

install: build
    install -Dm0755 applet/target/release/cosmic-applet-hue {{bin}}
    sed 's|^Exec=.*|Exec={{bin}}|' applet/res/{{appid}}.desktop > /tmp/{{appid}}.desktop
    install -Dm0644 /tmp/{{appid}}.desktop {{share}}/applications/{{appid}}.desktop
    rm -f /tmp/{{appid}}.desktop
    icon=applet/res/icons/local/cosmic-hue-symbolic.svg; [ -f "$icon" ] || icon=applet/res/icons/cosmic-hue-symbolic.svg; install -Dm0644 "$icon" {{share}}/icons/hicolor/scalable/apps/{{appid}}-symbolic.svg
    rm -rf {{share}}/cosmic-hue/backend
    mkdir -p {{share}}/cosmic-hue
    cp -r backend {{share}}/cosmic-hue/backend
    find {{share}}/cosmic-hue/backend -name __pycache__ -prune -exec rm -rf {} +

uninstall:
    rm -f {{bin}} {{share}}/applications/{{appid}}.desktop {{share}}/icons/hicolor/scalable/apps/{{appid}}-symbolic.svg
    rm -rf {{share}}/cosmic-hue
