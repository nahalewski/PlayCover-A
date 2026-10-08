.text
.globl _main
.p2align 2
_initialize:
    mov x16, #20 // initializer must dispatch getpid, then resume to ret
    svc #0x80
    b.cs init_failed
    adrp x8, _observed_pid@PAGE
    str x0, [x8, _observed_pid@PAGEOFF]
    ret
init_failed:
    mov x0, #102
    mov x16, #1
    svc #0x80
    brk #0
_main:
    adrp x8, _observed_pid@PAGE
    ldr x0, [x8, _observed_pid@PAGEOFF]
    cmp x0, #1001 // current virtual process PID
    b.ne failed
    mov x0, #42
    mov x16, #1
    svc #0x80
    brk #0
failed:
    mov x0, #103
    mov x16, #1
    svc #0x80
    brk #0
.data
.p2align 3
_observed_pid:
    .quad 0
.section __DATA,__mod_init_func,mod_init_funcs
.p2align 3
    .quad _initialize
