# orbe: scenes

Load this when writing a scene key, a part name, a param or `world`. `orbe skills get core` has the loop and one scene that works.

A scene is JSONC: JSON plus `//` comments and trailing commas.

- `format` is always `1`.
- `parts` names each instance. `type` is a part id or a `./` path to a part folder of your own. Other keys are that part's params. `"not_simulated": "<why>"` keeps a part for drawing only. Nothing may be wired to it. A wire to one of its pins is `error[not_simulated]` and the message gives `<why>`.
- `wires` pairs pins, as `["nano.D2", "btn1.a"]`. Wired pins join into nets.
- `nets` (optional) names a net by one of its pins, `"GND": "nano.GND"`. The ground net is named `GND` when `nets` does not give that name to another net. A hole never names a net. Every other net takes its smallest pin, with `.` written `__`.
- `layout` (optional) seats and places parts. Load `orbe skills get breadboards` for `on` and `at`.
- `ground` (optional) is a pin on the ground net, as `"ground": "bb.X1"`, when no board names one. The net is still `GND` when that name is free.
- `sim` (optional) is `{ "max_step": "1ms" }`. The engine's largest step is 10 ms when you omit it.
- `world` (optional) is the room, the same for every part. Each quantity is a number, or `{ "initial": 25, "steps": [{ "at": "2s", "to": 30 }] }`.

| Quantity | Unit | Range | Default |
| --- | --- | --- | --- |
| `temperature` | degC | -40 to 125 | 25 |
| `humidity` | %RH | 0 to 100 | 50 |
| `light` | lx | 0 to 100000 | 300 |
| `pressure` | Pa | 30000 to 110000 | 101325 |

A part name starts with a letter and then takes letters, digits and `_`. It may not contain `__` or end in `_`. `i`, `v`, `p`, `ib`, `ic`, `id`, `iin`, `iout`, `pin`, `logic`, `usart` and `net` are taken.

A param is a number with an optional prefix (`p`, `n`, `u`, `m`, `k`, `M`) and a unit, such as `"10k"`, `"100n"` or `"1ms"`, a choice by name, as `"color": "red"`, or a path from the scene's folder, such as the firmware. The part reads that file as text, or as bytes when the param says `"as": "blob"`. A 5 mm or 3 mm LED colour is its own part: `led-5mm` is red and `led-5mm-blue` is blue, and neither takes `color`. A 0603 or 0805 still takes `color`. A capacitor starts at the circuit's DC solution, which on a pin that is still an input is not 0 V. `"initial": "0"` starts it at 0 V.

`orbe set light 50` or `orbe set temperature 40` changes a world quantity from then on (`thermistor-ntc-10k` follows `temperature`). `orbe get light` and `orbe get @world.light` read the value in force. Load `orbe skills get sessions-and-time` for when a `set` lands.
