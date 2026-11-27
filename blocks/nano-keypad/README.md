# 4x4 membrane keypad on a Nano

A 4x4 membrane matrix keypad (`keypad-4x4`) is wired to an Arduino Nano: rows `R1` to `R4` on `D11` to `D8`, and columns `C1` to `C4` on `D5` to `D2`.

The sketch runs the stock Arduino `Keypad` library (3.1.1), prints `ready` at 115200 baud, prints `key=<char>` whenever a key is pressed, and lights the Nano's `L` LED while a key is held.

Drive it from the CLI with `orbe press @pad.k1`, `orbe release @pad.k1`, `@pad.k5`, `@pad.khash` (and `k1..k9`, `k0`, `ka..kd`, `kstar`), or click any key cap directly in the 3D viewer.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
