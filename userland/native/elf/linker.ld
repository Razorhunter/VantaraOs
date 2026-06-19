ENTRY(_start)

SECTIONS
{
    . = 0x1001000;

    .text : {
        *(.text .text.*)
    }

    . = ALIGN(0x1000);
    .rodata : {
        *(.rodata .rodata.*)
        *(.eh_frame_hdr)
        *(.eh_frame)
    }

    . = ALIGN(0x1000);
    .data : {
        *(.got)
        *(.got.plt)
        *(.data .data.*)
    }

    . = ALIGN(0x1000);
    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    }
}
