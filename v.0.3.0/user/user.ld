/* user.ld — link woflOS userspace programs at USER_CODE_VA = 0x400000.
 *
 * Fixed-VA + static reloc: every address resolved at link time, so the kernel
 * loader stages raw bytes with ZERO runtime relocation. Each section is
 * page-aligned so the loader maps whole 4 KiB pages with per-section W^X flags
 * (.text R+X, .rodata R, .data/.bss R+W). Boundary symbols are extracted by
 * build.sh into user_layout.rs — the linker script IS the loader's contract.
 *
 * VALIDATED (riscv64-unknown-elf-ld): entry=0x400000, _start first, sections
 * page-aligned & contiguous, objcopy flat size = text+rodata+data, bss stripped.
 */
ENTRY(_start)
USER_BASE = 0x400000;   /* MUST equal kernel process::USER_CODE_VA */

SECTIONS
{
    . = USER_BASE;

    .text : ALIGN(4096) {
        __text_start = .;
        KEEP(*(.text.start))    /* _start FIRST -> USER_BASE lands on it */
        *(.text .text.*)
        . = ALIGN(4096);
        __text_end = .;
    }
    .rodata : ALIGN(4096) {
        __rodata_start = .;
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
        . = ALIGN(4096);
        __rodata_end = .;
    }
    .data : ALIGN(4096) {
        __data_start = .;
        *(.data .data.*)
        *(.sdata .sdata.*)
        . = ALIGN(4096);
        __data_end = .;
    }
    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        . = ALIGN(4096);
        __bss_end = .;
    }
    /DISCARD/ : {
        *(.eh_frame .eh_frame_hdr)
        *(.comment) *(.riscv.attributes)
        *(.got .got.*)          /* static reloc -> none expected; drop defensively */
    }
}
