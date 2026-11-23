# orbe: errors

Load this when an error code is not in the troubleshooting table in `orbe skills get core`. Do what the `hint:` says. Part and `orbe add` codes are in `orbe skills get parts`.

With `--json` a command prints one object, `{"ok": true, "data": {...}}` or `{"ok": false, "error": {"code", "message", "hint"}}`. A failed `orbe check scene.jsonc --json` uses the same `error.code` as `orbe open --json`: `part_missing` or `firmware_missing` when every finding has that code, and `scene_invalid` otherwise. Each finding, its code and hint are in `data.diagnostics`. Live commands also add `session`, `t_ps` and `state`. Load `orbe skills get sessions-and-time` for the states.

## Errors

| Code | What to do |
| --- | --- |
| `usage` | The command line is wrong. The message says how. |
| `firmware_missing` | Build the firmware, or fix the path. It starts from the scene's folder. |
| `not_simulated` | A wire reaches a pin the engine does not simulate, or a part marked `not_simulated`. Wire another pin, or remove the wire. Load `orbe skills get not-simulated`. |
| `net.short` | A wire joins the two ends of a supply. Remove it. A load across that supply needs a part that limits the current, such as a resistor. |
| `scene.unknown_pin`, `scene.param`, `scene.syntax` | A typo or a wrong value at that line and column. The hint names what is valid. |
| `scene.world` | A `world` quantity that does not exist, a value outside its range, or a step at 0, too close to the one before it, or that does not change the value. |
| `no_session` | No session runs under this name. Run the `orbe open` in the hint. |
| `session_crashed` | The session's process died. Run `orbe open` again. It names `<runtime>/orbe/<name>/<pid>.crashed` when that process left output. A crash that wrote nothing does not name an older crash file, and that file stays. Exit 4. |
| `version_mismatch` | Another version of orbe started this session. Run `orbe close`. |
| `unknown_ref`, `no_reading` | The ref names nothing, or nothing to read. The hint lists what exists. |
| `not_settable` | `press` and `release` work only on a switch. `set` works on a quantity the scene's `world` names or a part reads, and on a live param. Other params are fixed. The hint names the live ones. |
| `value` | A live param set out of range. |
| `wrong_setting` | The part takes a different input, such as a `press` on a part that is not a switch. |
| `no_change` | `press` or `release` asks for the switch state already there. `set` of the value already in force succeeds, and names the time when that change is still ahead. A world `set` lands now and drops that quantity's later steps. |
| `expect_failed`, `until_timeout` | The condition did not hold. Compare `actual`, then fix the circuit or the firmware, or step longer. |
| `ended` | The session reached its 2-hour stop. Reads still work. Run `orbe open` to start again. |
| `past_stop` | A `step` would end past the stop, or an input would land past it. Do what the hint says. |
| `cancelled` | Ctrl-C stopped a `step`. The session stays. Step again. `orbe close` during a `step` ends the session. The message says it was closed, and the hint says how to open it. |
| `faulted` | The engine stopped the session. The message names why. Fix it and run `orbe open`. |

A chip error such as `chip.unsupported_setting` names the peripheral and the setting. Load `orbe skills get arduino`.
