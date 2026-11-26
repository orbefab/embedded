# 4xAA 6 V battery pack and SG90 servo on a Nano

A 4xAA 6 V alkaline battery pack (`battery-aa-4x`, 6.0 V EMF, 0.9 Ω internal resistance, 1.0 A rating) powers an `servo-sg90` micro servo on its own 6 V rail, sharing only ground with an Arduino Nano on USB. A 10 kΩ / 10 kΩ divider brings half of the pack voltage into `A0`.

While the servo holds (5 mA draw), the pack sits at 5.99 V. While the servo moves (140 mA draw), the 0.9 Ω pack internal resistance sags the rail by 0.126 V to about 5.87 V — staying comfortably inside the SG90's 4.5 to 6.0 V operating range without loading USB.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
