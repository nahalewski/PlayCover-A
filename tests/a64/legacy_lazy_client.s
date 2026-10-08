// LLD emits __stubs, __la_symbol_ptr and legacy lazy-bind records for this call.
.text
.global _main
_main:
    stp x29, x30, [sp, #-16]!
    mov x29, sp
    bl _answer
    ldp x29, x30, [sp], #16
    ret
