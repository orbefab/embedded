# ATmega328P core

`crates/avr`, package `sim-avr`, is a cycle-counted model of the ATmega328P. The circuit crate holds it behind the active chip seam, and each `atmega328p` part of a netlist runs one. It has no dependencies.

The sources are two documents. DS40002061B is the datasheet, written DS here. DS40002198B is the instruction set manual, written ISM, and its AVRe column gives the cycle counts. A comment in the crate cites one of them, by section, table, or figure.

## What it models

A `Cpu` is one chip. `parse_hex` or `parse_elf` reads an image and `Cpu::load_image` puts it in flash. `Cpu::run_until` runs to a cycle, `Cpu::take_pin` yields each pin change with the cycle it happened on, and `Cpu::drive_pin` drives a pad from outside. `Cpu` is `Clone`, and a clone shares the flash. Cycle 0 is the first instruction, and power-on start-up is already over at that point.

A pin report carries the pin's level and its drive, the pin's own stage apart from the host: `High` or `Low` for an output, `PullUp` for an input with `PORTxn` set and `PUD` clear, and `Off` for an input with no pull-up (Table 14-1). A report goes out when either changes, so a pull-up that turns on under a pin the host already holds high is still a report. While `RSTDISBL` is unprogrammed, `PC6` is `RESET`: its drive is always `PullUp`, its port bits do not move it, and `PINC6` reads 0 (Table 14-7, §14.3.2). An in-run reset keeps what the host drives on the pads.

- The instruction set the datasheet lists for this die, with the hardware multiplier, `MOVW`, and the enhanced `LPM`. Interrupts are Table 12-6, 26 vectors of two words each. Entry takes four cycles (§7.7.1).
- Ports B, C, and D, with the synchronizer on `PINx` (§14.2.4), the pull-ups, and the alternate functions of §14.3 on one pin mux. A pin id is port × 8 + bit, so `PB0` is 0, `PC0` is 8, and `PD0` is 16.
- Timer0, Timer1, and Timer2 in every waveform mode, the shared prescaler of §17, input capture with the noise canceler, and the `T0`/`T1` clock inputs.
- `INT0`, `INT1`, and the three pin-change groups (§13).
- USART0 in asynchronous mode and as an SPI master (§20, §21). SPI master and slave (§19). The TWI master (§22).
- The ADC and the analog comparator (§23, §24), sampling voltages the host sets.
- EEPROM (§8), the watchdog (§11), every reset source with its `MCUSR` flag, brown-out, the sleep modes and their wake sources (§10), `PRR`, `CLKPR`, and the fuse bytes.

A timer's next compare, TOP, or BOTTOM is a cycle number, and a read of `TCNT` works the count out from the cycle. An instruction that ends before that cycle does not step the timer.

`Cpu::run_until` and `Cpu::run_to_news` retire runs of register-only instructions without per-instruction bus work. An op is free when it reads and writes only registers, `SREG` other than setting `I`, `PC` and the cycle count. Loads, stores, stack ops, calls and returns, the I/O bit ops, `SEI`, `SPM`, `SLEEP`, `WDR` and `BREAK` are not free. The run loop first checks three things. The bus must be quiet: no `TCNT` hold, no store waiting for its stamp, no enable window, no CPU halt and no pin report waiting to be flushed. Every instruction's bus tick leaves the bus quiet, so this check guards a `Cpu::poke` between runs, such as one that sets `EERE` and halts the CPU before the next instruction. No interrupt can be taken: none is pending, or `I` is clear. Under `run_to_news`, no news waits. If all three hold, the core runs free ops back to back, and the bus cycle moves once by their total. The op that reaches the next device event or the limit retires the usual way, through the bus tick, the watchdog and sleep checks, and the interrupt check. The first op that is not free ends the run and steps as usual. No state the host or the firmware can read changes, for these reasons:

- Device state, interrupt lines, the watchdog, sleep, pin reports and the news change only at a device event, on a bus access, or on a host call between runs. A run stops before the first bus access, and at the op that reaches the next event.
- A free op that ends before the next event takes the tick's fast path. On a quiet bus that path only moves the cycle, so one move by the sum is the same.
- A free op can clear `I` but not set it. So an interrupt that could not be taken when the run began cannot become takeable inside it.
- `SEI` and `RETI` are not free. An interrupt held by their one-instruction delay is pending with `I` set, so the run does not start, and the next instruction steps alone before the entry.

`tests/free_runs.rs` compares both runs with a loop of single `Cpu::step` calls. It checks an interrupt, a timer event and a run limit at every cycle offset of a free run, `SEI` with an interrupt pending, a run that starts just after `SEI` or `RETI`, a host poke of `EERE` awake and before a wake, a byte stamped just before a free run, and the shipped firmware with the interrupt-heavy timing programs.

## Model parameters

The documents leave four timing points open or spread across figures. The core fixes each one.

1. **A clock-select store takes effect at the instruction's retiring edge.** A store lands on the clock edge that ends its instruction (Figure 14-4), and the timer is a synchronous design clocked through an enable (§15.8). So every timer clock up to and including that edge runs with the old select, and the new one counts from the next edge. Those clocks do everything a timer clock does: they count, raise `TOV` and the delayed compare flag, drive the OC pin, and copy a buffered `OCR`. An `OUT` to `TCCR0B` that ends at edge 8 gives a /1 timer its first count at edge 9. A `GTCCR` store resets the prescaler on the same edge, after the timers have run their clock there, so a `PSRSYNC` store that ends at edge E gives a /N timer its next count at E + N (§17.2, §17.4.1).
2. **A flag raised on an instruction's last cycle is served at that boundary.** §7.7.1 finishes a multi-cycle instruction before the interrupt is served. A flag that rises on the edge that ends an instruction is pending at that boundary, so the next thing the core does is the four-cycle entry. A flag that rises inside a two-cycle instruction waits for it to finish, so an interrupt period is one cycle longer or shorter than the timer's, by where the program's loop is when each flag rises.
3. **The prescaler counts 0 at the first instruction.** §17.2 makes the prescaler free running and puts the first count 1 to N+1 cycles after the timer is enabled, but gives no phase. The core starts the count at cycle 0, and again at the cycle a reset releases. So the first /64 count is cycle 64.
4. **The OC pin changes on the clock where TCNT reaches OCR, and the flag one timer clock later.** §15.5 sets the compare flag "at the next timer clock cycle" after the match, and Table 15-3 drives the pin from the match itself. Timer1 and Timer2 follow the same rule (§16.7, §18.5). In CTC with OCR0A = 99 at /1, OC0A toggles on the 99th count. In fast PWM a non-inverting OC0A is high while TCNT0 runs 0 to 98, which is 99 counts. The two extremes follow the datasheet's own cases. OCR0A = BOTTOM is the narrow spike of §15.7.3, one timer clock high each period. OCR0A = MAX is a constant output (§15.7.3, Table 15-3 note 1): the match is ignored and the pin sets at BOTTOM. So OCR0A = 0 and OCR0A = 1 give the same waveform, one clock high, and OCR0A = 254 is 254 clocks high while 255 never falls. In phase-correct PWM, a non-inverting pin is continuously low at OCR = 0 and continuously high at OCR = TOP, and an inverted pin is the opposite (§15.7.4, §16.9.4, §18.7.4). The double buffer copies at TOP (§16.9.4). Timer0 and Timer2 state the clear when OCR leaves TOP (Figure 15-7, Figure 18-7). §16.9.4 does not, and Timer1 follows them. A store that drops OCR below TOP clears a non-inverting pin on the TOP that copies the buffer, the first TOP after the store.

Other choices that the documents do not settle are written beside the code that makes them. The watchdog count starts at the store that sets `WDE`. SRAM is zero at power-on. The SPI clock's first edge is half a period after the `SPDR` store. The adapter writes Table 30-1's 1 uA leakage bound as a resistance from each port pin to each rail, so the pin leaks to both rails alike.

## Not yet modelled

- **TWI on the pins.** The TWI is the transaction, with the status codes of Tables 22-2 and 22-3 and the timing of §22.5.2. SDA and SCL show only their pull-up level, and no edge leaves the pads. Slave mode is not modelled.
- **SPI with a part on a net.** SPI drives SCK, MOSI, and MISO, but its peer is inside the crate: a byte queue that answers a master, or a host master that clocks a slave. So are the serial and I2C peers. They stand in as the test host until the adapter to the engine's nets replaces them.
- **The analog comparator through the seam.** The adapter hands the core each pin as a level from its watch, so `AIN0`, `AIN1`, and an ADC channel the comparator reads through `ACME` compare 0 V or VCC, not the pins' voltages.
- **`tRST`.** Table 29-11 (§29) gives `RESET` a minimum pulse width of 2.5 us. The adapter does not filter it and resets on a low of any length.
- Timer2's asynchronous clock from a TOSC crystal. `ASSR.AS2` is stored and logged, and Timer2 counts `clkI/O`.
- The boot section. The image runs from word 0, so parse rejects a programmed `BOOTRST`, and `SPM` is logged but does not write flash.

## Behind the seam

`Timeline` runs the core on a host's clock. Its time is the oscillator's cycle count from power-on, which no `CLKPR` write changes, so the host converts it with the one crystal rate. Inside the crate each pin report converts through the `CLKPR` epoch in force at its cycle. `Cpu::run_to_news(limit)` returns after the first step that leaves a pin report, an ADC sample the host must answer, or a new division, and a start-up, a held reset, or a sleep coasts no further than `limit`. `Timeline::run_until(limit)` halts at the first oscillator cycle in `(reached, limit]` where a pin's drive changes or the ADC samples. The core runs whole instructions and may sit a few cycles past a halt, but each drive change keeps its own cycle, so a run split at any limit is the same run.

§9.11 puts a new `CLKPR` division in force between `T1 + T2` and `T1 + 2 * T2` after the write, where `T1` is the old period and `T2` the new one, and two active clock edges fall in that interval. The core runs the `STS` that writes it and one more CPU cycle at the old division, then counts at the new one. The two edges after the store retires are then `T1` and `T1 + T2` after it, so the second is on the window's opening whether the clock slows down or speeds up. In `firmware/clkpr.S`, /1 to /2, the store retires at oscillator cycle 8, the window is 11 to 13, and the `SBI` after it toggles PB5 at 11. In `firmware/speedup.S` under `CKDIV8`, /8 to /1, the store retires at oscillator cycle 64, the window is 73 to 74, and the toggle is at 73. The new division begins a cycle past the one the core stops on after the store, so the core converts between CPU and oscillator cycles across its last two divisions.

The ADC asks the host for its volts at sample-and-hold. The timeline halts there, and the host answers before the conversion completes. An input reaches the pad at the CPU cycle the timeline reached, whatever instruction the core ran across that cycle, a `CLKPR` store included, so the synchronizer, `INT0` and `INT1`, the pin-change groups, input capture, and the `T0` and `T1` inputs all count from that cycle (§14.2.4: only the synchronizer adds latency). What they made by the cycle the core sits on, a flag, `ICR1`, or a count, is in place before the next instruction, and an interrupt it raised is taken at that boundary unless `SEI` or `RETI` just ran (§7.7.1). The host lands every input of one instant before the core takes one, so the lowest vector among them runs first. What an input causes comes at least one CPU cycle after where the timeline reached.

Two inputs still land where the core sits, up to an instruction's length later. A read inside the instruction that ran past the input's cycle, of `PINx` or of a flag, has already read the old value. A low on `RESET` and a brown-out reset the chip at the instruction boundary the core is on.

`crates/circuit/src/active/avr.rs` is the adapter, `active::Core::Avr`. It builds its chip from a netlist's `atmega328p` part, which [netlist.md](netlist.md#atmega328p) defines: the 28 package pins in DIP order, the output stages, both pull-ups, the I/O and `RESET` input thresholds, the crystal in hertz, the fuse bytes, and the flash image. `PACKAGE` gives each package pin's kind in the order of the part's pin names, and every lookup derives from it. Parse admits only the fuse settings the model runs, as [Fuses](#fuses) says, so the clock is a crystal or ceramic resonator (Table 9-1, `CKSEL` 0110 to 1111), and the crystal holds `PB6` and `PB7`. The other 21 port pins, `PC6` among them, are each an output in the image and a watch, in package order.

- A drive maps to the seam's drive one for one. `PullUp` stamps the pin's pull-up to the chip's own VCC pin, the reset pull-up on `PC6` while it is `RESET`.
- Every port pin, `PC6` as `RESET` included, stamps its leakage to the chip's own VCC pin and again to its GND pin in every drive state. So each port pin's DC paths to both rails, which `net.floating` counts, are in every solve.
- A watch reads its pin against fractions of the chip's own supply, `V(pin) - f * V(VCC) - (1 - f) * V(GND_8)`. It rises above the high fraction and falls at or below the low one. A flip drives the pad at the CPU cycle of its picosecond, as above. `PC6` while it is `RESET` reads against `RESET`'s own thresholds, and low holds the chip in reset and high releases it.
- With brown-out fused on, one more watch reads `VCC - GND_8` against the fuse's `VBOT+` rising and `VBOT-` falling (Tables 29-11 and 29-12). Below the window the chip holds in reset and every port pin is off.
- Each watch is a logic signal, its level the one the core takes. A port pin's is named for the pin, so `PC6` keeps its name while it is `RESET`, and the brown-out watch's is `VCC`. They are in name order, `PB0` to `PD7`, then `VCC` while brown-out is fused on, the order of their refs. The brown-out watch has a signal because it is the comparator that resets the chip, and `#net.vcc` alone does not show when its window, 25 mV either side of `VBOT`, trips.
- The samples are ADC0 to ADC5, `AVCC`, and `AREF`, each against `GND_22`, read on the step's solution at the stop, before any change there.
- A run to `limit_ps` runs the timeline to oscillator cycle `floor(limit_ps * f / 1e12)`, and a halt at cycle `c` is a stop at `ceil(c * 1e12 / f)` ps.

The values are the part's own fields. [netlist.md](netlist.md#atmega328p) gives their defaults and Table 30-1's wider limits. Each default is a typical eye reading of a DS figure but `leakage_resistance`, which is derived from a bound.

The tests run firmware written for the repo. `firmware/*.S` is the source, and the committed `firmware/*.hex` is what they read. `scripts/firmware.sh` rebuilds the hex with `avr-gcc` and `avr-objcopy`, and a build of the engine never needs them.

Ten seconds of `firmware/blink.S` at 16 MHz are 21 stops and 20 edges on PB5. The first stop, at cycle 2, is `SBI DDRB,5` turning PB5 from off to driven low. It changes the image, and on a pin its load already holds low it does not change the level. `blink_throughput` (ignored, in `crates/circuit/src/tran/tests/avr.rs`) times those 160e6 cycles on the bare core.

`examples/atmega-blink` runs `firmware/blink.S` for 4 s with an LED on PB5, and `crates/cli/tests/blink.rs` checks its edges and the LED's current. Its 64e6 core cycles are most of a release `sim run` of it.

`examples/atmega-button` runs `firmware/button.S` for 1 s with that LED and a push button from PD2 to ground. The pin's pull-up holds PD2 high while the button is open, and the loop copies the inverse of PIND2 to PB5. `crates/cli/tests/button.rs` checks those edges and the LED's current. Each pass reads two ports and writes `PORTB`. A store that changes neither DDR nor PORT does no pin work, unless a poke planted the wires since the last reconcile. `button_throughput` times its 32e6 cycles on the bare core and through the seam.

`examples/atmega-analog-serial` runs `firmware/analog_serial.S` for 19 ms. A 10 kohm potentiometer sits across VCC and ground with its wiper on PC0. The firmware reads ADC0 against AVCC and prints the code as decimal on TXD, one line per reading. `U2X0` stays clear and `UBRR0` is 103, so a bit is 16 * (103 + 1) cycles, 104 us at 16 MHz (Table 20-1 and Table 20-6). The run prints three lines. `crates/cli/tests/analog_serial.rs` checks the wiper voltage and those frames. On the build machine a release `sim run` takes 0.028 to 0.033 s over seven runs, alternating with `examples/divider`, which takes 0.0015 to 0.0023 s. This run's time is user CPU. Serial output is the TXD waveform in `logic.vcd`. A decoded serial file is not part of the run folder.

`examples/atmega-fade` runs `firmware/fade.S` for 1.57 s with the WP7113ID of `examples/led-5v` on PB1. Timer1 is 8-bit phase-correct PWM, prescaler 64, a period of 32640 cycles, 2.04 ms at 16 MHz (§16.9.4). `OCR1A` steps by 5 every 480,000 cycles, 30 ms, and reverses at 0 and 255. `crates/cli/tests/fade.rs` checks the PB1 edges and the LED's current. The run writes 15701 rows and 1481 PB1 edges. Release time is user CPU on one core. On the build machine a release `sim run` takes 0.82 to 0.83 s over five runs. [bench.md](bench.md) times the committed run and the `live` run of this example.

## Fuses

Parse keeps only the fuse settings the model runs, and [netlist.md](netlist.md#atmega328p) lists the rejected ones and why. The core reads these.

- `CKSEL0` and `SUT` pick the crystal's row of Table 9-4 or 9-6: the delay after a reset, and the start-up from power-down and power-save. `CKSEL3:1` picks among the crystal rows of Table 9-1, which the model runs alike. Power-on start-up is over at cycle 0.
- `CKDIV8` programmed starts `CLKPS` at /8 (§9.12.2), so the CPU runs at an eighth of the crystal until the firmware writes `CLKPR`.
- `RSTDISBL` programmed makes `PC6` a port pin (§14.3.2).
- `WDTON` programmed holds the watchdog in system reset mode (Table 11-1).
- `BODLEVEL` 110, 101, or 100 enables brown-out at Table 29-12's typical `VBOT`, 1.8, 2.7, or 4.3 V, with `VBOT-` and `VBOT+` 25 mV either side, half of Table 29-11's typical `VHYST`. 111 disables it.

## Ratings

§29.1 rates the part at 40 mA DC through any one I/O pin and 200 mA DC through the `VCC` and `GND` pins, as absolute maxima. It limits the voltage on any pin but `RESET` to -0.5 V to VCC + 0.5 V with respect to ground, and on `RESET` to -0.5 V to 13.0 V. The netlist kind has no `ratings` object yet, and the run does not check these. A later change checks them.

## The timing set

`crates/avr/tests/timing.rs` is the pass test. Thirty-nine programs written for it are in `tests/timing/asm`, and `tests/timing/hex` holds their builds for the ATmega328P, with no start files or libraries. The header of `timing.rs` gives the command that builds them, and every committed image is byte-identical to a fresh build of its source. Each program marks time with `out PORTB`, which drives the pin on the edge that ends the `out` (Figure 14-4). The test runs each image, reads the pin edges, and checks each row against the value the documents give. The rows cover instruction cycles, pin writes, interrupt entry and return, Timer0 and Timer1 overflow periods, CTC and fast-PWM waveforms with OCR0A at 0, 1, 99, and 0xFF, the compare-to-overflow lag, and clock-select timing. The timing set does not row-check phase-correct PWM, and `crates/avr/tests/timers.rs` and `crates/cli/tests/fade.rs` do.

Every row carries its citation and is exact. There are 107 rows. The interrupt periods and the compare-to-interrupt lags are worked out from each program's loop phase under parameter 2, in `IsrLoop`. A model change that moves a row fails the suite, and a row's expected value moves only when the document reading behind it does.

`cargo run --release -p sim-avr --example throughput` reports how fast the core runs a timing-set program on the build machine. `scripts/check.sh` does not run it.
