# 2-axis thumb joystick on a Nano

A KY-023 2-axis thumb joystick module (`joystick-ky023`) is wired to an Arduino Nano: `VCC` to 5 V, `GND` to ground, `VRX` to `A0`, `VRY` to `A1`, and `SW` to `D2` (`INPUT_PULLUP`, active-low).

The sketch prints `x=<0..1023> y=<0..1023> btn=<0|1>` over Serial at 115200 baud at startup and whenever either axis moves or the button changes, and lights the Nano's `L` LED while the thumbstick is pressed.

Drive it from the CLI with `orbe set @joy.x.position 0.9`, `orbe set @joy.y.position 0.1`, `orbe press @joy.sw` and `orbe release @joy.sw`, or drag the sliders and click the thumbstick cap in the 3D viewer.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
