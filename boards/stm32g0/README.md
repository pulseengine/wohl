# `boards/stm32g0`

Hardware board notes for STM32G0-class sensor nodes in the Wohl home
supervision system.

This directory holds **board-level documentation and linker artifacts**
only — no Rust code. The firmware itself lives in
[`crates/wohl-fw-door/`](../../crates/wohl-fw-door/).

## Role

STM32G031 is the architect's choice (see
[`spar/wohl_system.aadl`](../../spar/wohl_system.aadl)) for the smallest,
cheapest sensor nodes:

> *Sensor nodes: ESP32-C3 or STM32G031 (Cortex-M0+, 64MHz, 8KB SRAM)*

For the **Door / Window** node — a reed switch glued to a doorframe —
STM32G031 in particular fits the bill:

- **Ultra-low quiescent current** in `STOP1`/`STANDBY` (door is shut most
  of the time, MCU sleeps until reed edge).
- **Cortex-M0+ / ARMv6-M** — Rust target `thumbv6m-none-eabi`.
- **8 KB SRAM, 32 KB Flash** — plenty for a 14-byte CCSDS encoder + UART
  driver + bootloader stub.
- **Tiny QFN-32 package** runs from a CR2032 coin cell.

The first revision targets the **STM32G031K8** (32 KB flash, 8 KB SRAM,
LQFP-32). Other G0 variants (G030, G031J6, G071) require only a
feature-flag swap in `wohl-fw-door`'s `Cargo.toml`.

## Pin mapping (STM32G031K8, LQFP-32)

| Pin    | Net         | Function                                  | Notes                                  |
|--------|-------------|-------------------------------------------|----------------------------------------|
| `PA9`  | `UART1_TX`  | USART1 TX → hub (115200 8N1)              | AF1                                    |
| `PA10` | `UART1_RX`  | USART1 RX from hub (currently unused)     | AF1                                    |
| `PA0`  | `REED`      | Reed-switch input, internal pull-up, EXTI | Closed = `0` (magnet present)          |
| `PC14` | `LED`       | Status LED (open-drain, optional)         | Low = on (saves cap on CR2032 droop)   |
| `PA13` | `SWDIO`     | SWD debug                                 | Reserved — do not reuse                |
| `PA14` | `SWCLK`     | SWD debug                                 | Reserved — do not reuse                |
| `VDD`  | `+3V0`      | CR2032 via low-Iq LDO (or direct)         |                                        |
| `VSS`  | `GND`       |                                           |                                        |

The reed switch is wired between `PA0` and `GND`; firmware enables the
internal pull-up, so `PA0` reads **high** when the door is open (magnet
absent, reed open) and **low** when the door is closed (magnet present,
reed shorted to ground). The CCSDS payload maps:

- `value = 0` → closed
- `value = 1` → open

…matching `SENSOR_CONTACT` semantics in
[`relay-ccsds::sensor_wire`](../../../relay/crates/relay-ccsds/plain/src/sensor_wire.rs).

## UART framing

CCSDS Space Packets are self-delimiting (the 6-byte header carries the
length field), so the firmware emits raw 14-byte packets back-to-back
on USART1 without any inter-packet marker. The hub's `--ccsds` consumer
re-synchronises on the next valid header if a byte is lost. Baud rate
is fixed at **115200 8N1** to match the hub's default UART config.

## Clocking

Default firmware boots on the internal **HSI16** (16 MHz). No external
crystal is fitted on the reference board; CR2032 cannot reliably drive
one through ageing. UART baud error at 115200 / 16 MHz is < 0.2 %, well
within the 2.5 % asynchronous tolerance.

## Linker / memory

The reference build uses `cortex-m-rt`'s default `link.x`. A
chip-specific `memory.x` will be added here once we standardise on a
single G0 variant for the production batch:

```text
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 32K
  RAM   : ORIGIN = 0x20000000, LENGTH = 8K
}
```

Until then `wohl-fw-door` ships its own `memory.x` for the G031K8 so
that `cargo build --release` produces a flashable ELF without any extra
configuration.

## Related AADL

- `spar/wohl_firmware.aadl` — `DoorFirmware` thread (sporadic, 2 ms WCET,
  100 ms deadline, 2 KB stack).
- `spar/wohl_nodes.aadl` — `DoorWindowNode` system (currently models the
  nRF52840 + Thread variant; an STM32G0 + wired-UART variant is the
  hardware target for this firmware).

## Out of scope for this directory

- Firmware code (lives in `crates/wohl-fw-door/`).
- Probe-rs / defmt-test wiring (follow-up — see issue tracker).
- Schematics / PCB Gerbers (separate `hardware/` repo).
