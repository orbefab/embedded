---
name: orbe
description: Simulates a circuit and its microcontroller firmware with the orbe CLI, such as a button and an LED on an Arduino Nano or Uno. Use to build, run and test a hardware project without the hardware, or when a project has a scene.jsonc or tests/*.test files.
---

# orbe

Run `orbe skills get core` and follow it before anything else. The `orbe` you have installed prints it, so it matches that version.

Before you write or build Arduino firmware, also run `orbe skills get arduino`.

`orbe part led-5mm` prints one catalog part. `orbe search led` finds parts by id, title, maker, number, summary and tags. Look a part up that way before you add it.

`orbe skills list` lists every skill. If `orbe` is not found, it is not installed. Install it, then run `orbe skills get core`:

```sh
curl -fsSL https://orbefab.com/embedded/install.sh | sh
```
