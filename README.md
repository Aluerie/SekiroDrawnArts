# ⚔️ SekiroDrawnArts

A Sekiro mod that keeps the combat art slot empty unless you hold a key, so Block + Attack can never fire a combat art by accident.

- **Hold Shift**: the selected combat art is equipped. Shift + Attack performs it; the mod presses Block for you. Block + Attack also works.
- **Release Shift**: the slot goes back to empty.
- **Hold Tab**: a wheel of your unlocked combat arts appears. Point at one with the mouse, then release Tab to select it. The camera stays still while the wheel is open. Releasing without pointing at anything keeps the current selection.

An art you equip through the in-game menu becomes the selected art. The same happens with the art equipped in a save when you load it.

Only the Steam version 1.06 is supported. The mod calls game functions at fixed addresses. Before hooking anything, it checks that those addresses fall inside the running game, and it doesn't load if they don't. That check doesn't catch every other version, so other versions can still crash.

## 📦 Install

This mod is loaded by [Mod Engine 3](https://github.com/garyttierney/me3) (me3). It isn't a `dinput8.dll` and doesn't work with Mod Engine 1.

The quickest way: download `SekiroDrawnArts-v<version>.zip` from the
[latest release](https://github.com/Aluerie/SekiroDrawnArts/releases/latest), unzip it anywhere, and double-click
`drawn_arts.me3` (or run `me3 launch --game sekiro --profile drawn_arts.me3` in that folder). The zip holds the DLL,
the settings and a ready-made profile. It doesn't hold the wheel's images; see [Wheel images](#wheel-images) to add them.

The release folder, like this repository, is laid out the way me3 expects:

```text
drawn_arts.me3        the me3 profile
drawn_arts/
├─ drawn_arts.ini     settings
├─ drawn_arts.dll     built, not committed
└─ icons/             extracted from the game, not committed
```

To build it yourself instead:

1. Build with `cargo build --release` and copy `target/release/drawn_arts.dll` into `drawn_arts/`.
2. Optionally, extract the combat art images as described in [Wheel images](#wheel-images).
3. Double-click `drawn_arts.me3`, or run `me3 launch --game sekiro --profile path/to/drawn_arts.me3`.

To install it elsewhere, copy `drawn_arts.me3` and the `drawn_arts/` folder together.

To combine it with other mods, copy the `[[natives]]` block from `drawn_arts.me3` into your own profile and fix the `path`. Keep the `initializer` line: me3 calls `drawn_arts_init` once the game is running, and that call installs the mod.

The mod writes `drawn_arts.log` next to the DLL.

## ⚙️ Configure

Edit `drawn_arts.ini` to change the hold key, the wheel key, the default art and which arts the wheel shows. The file lists the key names and combat art ids.

The game still sees the hold key and the wheel key, so pick keys the game doesn't use or unbind them in the game's settings. For example, if Shift dodges in your keybinds, every Shift press also dodges.

<a id="wheel-images"></a>

### 🖼️ Wheel images

The wheel shows a picture for every combat art that has a PNG in `drawn_arts/icons/`, named after the art's id from `drawn_arts.ini`. For example, `icons/5700.png` is Mortal Draw. Arts without an image show only their name. The images are read once, when the game starts.

To use the game's own icons, unpack `menu/hi/01_common.tpf.dcx` and `menu/hi/01_common.sblytbnd.dcx` with [WitchyBND](https://github.com/ividyon/WitchyBND), then run:

```
pip install pillow
python tools/extract_icons.py <01_common-tpf-dcx folder> <01_common-sblytbnd-dcx folder> drawn_arts/icons
```

## 🛠️ Development

```sh
cargo test            # config parsing and wheel aiming tests
cargo build --release # target/release/drawn_arts.dll
```

To get the ready-to-go folder that players download, run `scripts\package.ps1`. It builds the DLL and creates
`dist\SekiroDrawnArts\` (the `.me3` profile, `drawn_arts\` with the DLL and settings, README) and
`dist\SekiroDrawnArts-v<version>.zip`. The wheel images are left out, because they're the game's own art.

### 🤖 CI

[.github/workflows/ci.yml](.github/workflows/ci.yml) runs on every push and pull request: `cargo fmt --check`,
`cargo clippy` with warnings as errors, `cargo test`, then the packaging script. The packaged folder can be downloaded
from the run's "Artifacts" section.

### 🚀 Releasing

1. Bump `version` in `Cargo.toml` and run `cargo build` so `Cargo.lock` picks it up. Commit both.
2. Tag the commit `v<version>`, e.g. `v0.2.0`, and push the tag.

CI then checks that the tag matches `Cargo.toml`, builds and tests everything, and publishes a GitHub Release with the
zip attached and release notes generated from the commits since the last tag.

## 🙏 Credits

Game addresses and the equip, inventory and input-hook approach come from [dec32/sekiro-battle-instinct](https://github.com/dec32/sekiro-battle-instinct), [tmsrise/Sekiro-Weapon-Wheel](https://www.nexusmods.com/sekiro/mods/1058) and [ReaperAnon/SekiroHotkeySystem](https://github.com/ReaperAnon/SekiroHotkeySystem).
