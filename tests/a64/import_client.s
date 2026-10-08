// Calls a chained import through a GOT slot and returns its result.
.text
.global _main
_main:
    stp x29, x30, [sp, #-16]!
    mov x29, sp
    adrp x8, _answer@GOTPAGE
    ldr x8, [x8, _answer@GOTPAGEOFF]
    blr x8
    ldp x29, x30, [sp], #16
    ret
