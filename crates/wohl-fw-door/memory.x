/* Linker memory map for STM32G031K8 (32 KB Flash, 8 KB SRAM).
 * Consumed by `cortex-m-rt`'s `link.x.in` via the `memory.x` search
 * path that `build.rs` adds to OUT_DIR. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 32K
  RAM   : ORIGIN = 0x20000000, LENGTH = 8K
}
