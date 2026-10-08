.text
.global _answer
_answer:
    adrp x8, _value@PAGE
    ldr x0, [x8, _value@PAGEOFF]
    ret
_initialize:
    adrp x8, _value@PAGE
    mov x0, #21
    str x0, [x8, _value@PAGEOFF]
    ret
_initialize_second:
    adrp x8, _value@PAGE
    ldr x0, [x8, _value@PAGEOFF]
    add x0, x0, x0
    str x0, [x8, _value@PAGEOFF]
    ret
.data
.p2align 3
_value:
    .quad 0
.section __DATA,__mod_init_func,mod_init_funcs
.p2align 3
    .quad _initialize
    .quad _initialize_second
