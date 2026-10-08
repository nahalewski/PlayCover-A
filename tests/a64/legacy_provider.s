// The data pointer must be rebased when this dylib is slid.
.text
.global _answer
_answer:
    adrp x8, _value_pointer@PAGE
    ldr x8, [x8, _value_pointer@PAGEOFF]
    ldr x0, [x8]
    ret

// The legacy lazy client imports this implicit symbol. Eager lazy-slot
// binding must bypass its stub helper; this sentinel must never run.
.global dyld_stub_binder
dyld_stub_binder:
    mov x0, #99
    ret

.data
.p2align 3
_value:
    .quad 42
_value_pointer:
    .quad _value
