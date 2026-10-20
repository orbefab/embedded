# ATmega328P core

`crates/avr`, package `sim-avr`, is a cycle-counted model of the ATmega328P. Nothing in a run uses it yet. It has no dependencies.

The sources are two documents. DS40002061B is the datasheet, written DS here. DS40002198B is the instruction set manual, written ISM, and its AVRe column gives the cycle counts. A comment in the crate cites one of them, by section, table, or figure.

## What it models

A `Cpu` is one chip. `parse_hex` or `parse_elf` reads an image and `Cpu::load_image` puts it in flash. `Cpu::run_until` runs to a cycle, `Cpu::take_pin` yields each pin change with the cycle it happened on, and `Cpu::drive_pin` drives a pad from outside. `Cpu` is `Clone`, and a clone shares the flash. Cycle 0 is the first instruction, and power-on start-up is already over at that point.

- The instruction set the datasheet lists for this die, with the hardware multiplier, `MOVW`, and the enhanced `LPM`. Interrupts are Table 12-6, 26 vectors of two words each. Entry takes four cycles (§7.7.1).
- Ports B, C, and D, with the synchronizer on `PINx` (§14.2.4), the pull-ups, and the alternate functions of §14.3 on one pin mux. A pin id is port × 8 + bit, so `PB0` is 0, `PC0` is 8, and `PD0` is 16.
- Timer0, Timer1, and Timer2 in every waveform mode, the shared prescaler of §17, input capture with the noise canceler, and the `T0`/`T1` clock inputs.
- `INT0`, `INT1`, and the three pin-change groups (§13).
- USART0 in asynchronous mode and as an SPI master (§20, §21). SPI master and slave (§19). The TWI master (§22).
- The ADC and the analog comparator (§23, §24), sampling voltages the host sets.
- EEPROM (§8), the watchdog (§11), every reset source with its `MCUSR` flag, brown-out, the sleep modes and their wake sources (§10), `PRR`, `CLKPR`, and the fuse bytes.

A timer's next compare, TOP, or BOTTOM is a cycle number, and a read of `TCNT` works the count out from the cycle. An instruction that ends before that cycle does not step the timer.

## Model parameters

The documents leave four timing points open or spread across figures. The core fixes each one.

1. **A clock-select store takes effect at the instruction's retiring edge.** A store lands on the clock edge that ends its instruction (Figure 14-4), and the timer is a synchronous design clocked through an enable (§15.8). So every timer clock up to and including that edge runs with the old select, and the new one counts from the next edge. Those clocks do everything a timer clock does: they count, raise `TOV` and the delayed compare flag, drive the OC pin, and copy a buffered `OCR`. An `OUT` to `TCCR0B` that ends at edge 8 gives a /1 timer its first count at edge 9. A `GTCCR` store resets the prescaler on the same edge, after the timers have run their clock there, so a `PSRSYNC` store that ends at edge E gives a /N timer its next count at E + N (§17.2, §17.4.1).
2. **A flag raised on an instruction's last cycle is served at that boundary.** §7.7.1 finishes a multi-cycle instruction before the interrupt is served. A flag that rises on the edge that ends an instruction is pending at that boundary, so the next thing the core does is the four-cycle entry. A flag that rises inside a two-cycle instruction waits for it to finish, so an interrupt period is one cycle longer or shorter than the timer's, by where the program's loop is when each flag rises.
3. **The prescaler counts 0 at the first instruction.** §17.2 makes the prescaler free running and puts the first count 1 to N+1 cycles after the timer is enabled, but gives no phase. The core starts the count at cycle 0, and again at the cycle a reset releases. So the first /64 count is cycle 64.
4. **The OC pin changes on the clock where TCNT reaches OCR, and the flag one timer clock later.** §15.5 sets the compare flag "at the next timer clock cycle" after the match, and Table 15-3 drives the pin from the match itself. Timer1 and Timer2 follow the same rule (§16.7, §18.5). In CTC with OCR0A = 99 at /1, OC0A toggles on the 99th count. In fast PWM a non-inverting OC0A is high while TCNT0 runs 0 to 98, which is 99 counts. The two extremes follow the datasheet's own cases. OCR0A = BOTTOM is the narrow spike of §15.7.3, one timer clock high each period. OCR0A = MAX is a constant output (§15.7.3, Table 15-3 note 1): the match is ignored and the pin sets at BOTTOM. So OCR0A = 0 and OCR0A = 1 give the same waveform, one clock high, and OCR0A = 254 is 254 clocks high while 255 never falls.

Other choices that the documents do not settle are written beside the code that makes them. The watchdog count starts at the store that sets `WDE`. SRAM is zero at power-on. The SPI clock's first edge is half a period after the `SPDR` store.

## Not yet modelled

- **TWI on the pins.** The TWI is the transaction, with the status codes of Tables 22-2 and 22-3 and the timing of §22.5.2. SDA and SCL show only their pull-up level, and no edge leaves the pads. Slave mode is not modelled.
- **SPI with a part on a net.** SPI drives SCK, MOSI, and MISO, but its peer is inside the crate: a byte queue that answers a master, or a host master that clocks a slave. So are the serial and I2C peers. They stand in as the test host until the adapter to the engine's nets replaces them.
- **The adapter to the engine.** Nothing turns a pin change into a net event or a net voltage into a pad level yet.
- Timer2's asynchronous clock from a TOSC crystal. `ASSR.AS2` is stored and logged, and Timer2 counts `clkI/O`.
- The boot section. The image runs from word 0, and `SPM` is logged but does not write flash.

## The timing set

`crates/avr/tests/timing.rs` is the pass test. Thirty-nine programs written for it are in `tests/timing/asm`, and `tests/timing/hex` holds their builds for the ATmega328P, with no start files or libraries. The header of `timing.rs` gives the command that builds them, and every committed image is byte-identical to a fresh build of its source. Each program marks time with `out PORTB`, which drives the pin on the edge that ends the `out` (Figure 14-4). The test runs each image, reads the pin edges, and checks each row against the value the documents give. The rows cover instruction cycles, pin writes, interrupt entry and return, Timer0 and Timer1 overflow periods, CTC and fast-PWM waveforms with OCR0A at 0, 1, 99, and 0xFF, the compare-to-overflow lag, and clock-select timing.

Every row carries its citation and is exact. There are 107 rows. The interrupt periods and the compare-to-interrupt lags are worked out from each program's loop phase under parameter 2, in `IsrLoop`. A model change that moves a row fails the suite, and a row's expected value moves only when the document reading behind it does.

`cargo run --release -p sim-avr --example throughput` reports how fast the core runs a timing-set program on the build machine. `scripts/check.sh` does not run it.
