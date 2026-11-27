# RGB LED colour mixing on a Nano

A 5 mm common-cathode RGB LED (`led-5mm-rgb-cc`, Kingbright WP154A4SUREQBFZGW) is driven by PWM from an Arduino Nano: D9 drives red through 220 Ω, D10 drives green through 150 Ω, and D11 drives blue through 150 Ω. The common cathode (`COM`, pin 2) goes to ground.

The sketch cycles through red, orange (red full, green half PWM), green, cyan, blue and magenta (red full, blue half PWM), holding each colour for 400 ms. Read each die separately with `orbe get @rgb.red`, `orbe get @rgb.green` and `orbe get @rgb.blue`.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
