# `boards/stm32g0`

Hardware board notes for the STM32G0 **bench/development** door sensor in
the Wohl home supervision system.

This directory holds **board-level documentation and linker artifacts**
only — no Rust code. The firmware itself lives in
[`crates/wohl-fw-door-bench/`](../../crates/wohl-fw-door-bench/).

## Role — bench tool, not a field node

This STM32G031 board is a **bench/development sensor**. It emits CCSDS
sensor packets over a wired point-to-point UART so the hub's `--ccsds`
ingest path can be exercised without radio or bus hardware.

It is **not** a field deployment. A real door/window sensor must be
wireless or on a multi-drop home-automation bus — point-to-point UART
does not scale to a house. The field door firmware targets:

- **STM32WL55** — sub-GHz (868/915 MHz) for wireless nodes
- **CAN-FD** — multi-drop wired bus for cabled installs

both carrying the same transport-agnostic 14-byte CCSDS payload. That
work is tracked separately in the issue tracker.

## Why STM32G031 for the bench tool

- **Cortex-M0+ / ARMv6-M** — Rust target `thumbv6m-none-eabi`.
- **8 KB SRAM, 64 KB Flash** (STM32G031K8) — ample for a 14-byte CCSDS
  encoder + UART driver.
- **Tiny QFN/LQFP-32 package**, cheap, USB/bench-powered.
- Stays in the STM32 family — same toolchain as the future STM32WL55 /
  CAN-FD field firmware.

## Pin mapping (STM32G031K8, LQFP-32)

| Pin    | Net         | Function                                  | Notes                                  |
|--------|-------------|-------------------------------------------|----------------------------------------|
| `PA9`  | `UART1_TX`  | USART1 TX → hub (115200 8N1)              | AF1                                    |
| `PA10` | `UART1_RX`  | USART1 RX from hub (currently unused)     | AF1                                    |
| `PA0`  | `REED`      | Reed-switch input, internal pull-up, EXTI | Closed = `0` (magnet present)          |
| `PC14` | `LED`       | Status LED (open-drain, optional)         | Low = on                               |
| `PA13` | `SWDIO`     | SWD debug                                 | Reserved — do not reuse                |
| `PA14` | `SWCLK`     | SWD debug                                 | Reserved — do not reuse                |
| `VDD`  | `+3V0`      | Bench/USB supply via LDO                  |                                        |
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
crystal is fitted; UART baud error at 115200 / 16 MHz is < 0.2 %, well
within the 2.5 % asynchronous tolerance.

## Linker / memory

`wohl-fw-door-bench` ships its own `memory.x` for the STM32G031K8 so
`cargo build --release` produces a flashable ELF without extra config:

```text
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 64K
  RAM   : ORIGIN = 0x20000000, LENGTH = 8K
}
```

(STM32G031**K8** is 64 KB flash / 8 KB SRAM — the `K8` suffix denotes
64 KB. Other G0 variants need only a `memory.x` + feature-flag swap.)

## Related AADL

- `spar/wohl_firmware.aadl` — `DoorFirmware` thread (sporadic, 2 ms WCET,
  100 ms deadline, 2 KB stack).
- `spar/wohl_nodes.aadl` — `DoorWindowNode.WiredG0` (this bench board:
  STM32G031 + wired UART) and `DoorWindowNode.Battery` (wireless field
  variant).
- `spar/wohl_hardware.aadl` — `processor STM32G031` + `SRAM_STM32G031` +
  `Flash_STM32G031`.

## Out of scope for this directory

- Firmware code (lives in `crates/wohl-fw-door-bench/`).
- Probe-rs / defmt-test wiring (follow-up — see issue tracker).
- STM32WL55 sub-GHz + CAN-FD field firmware (separate tracked effort).
- Schematics / PCB Gerbers (separate `hardware/` repo).
