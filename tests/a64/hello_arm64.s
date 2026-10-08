// EXPERIMENTAL freestanding arm64 "hello world" for the A64 prototype.
// Built into a static arm64 Mach-O by dev-scripts/build-a64-test-payloads.sh
// (NDK clang --target=arm64-apple-ios14.0 + ld64.lld). No libSystem/dyld:
// it talks to the "kernel" directly using the Darwin arm64 syscall ABI
// (x16 = syscall number, svc #0x80): 4 = write(fd, buf, len), 1 = exit(code).
    .section __TEXT,__text,regular,pure_instructions
    .globl _start
    .p2align 2
_start:
    stp     x29, x30, [sp, #-16]!
    mov     x29, sp

    // sum = sum_to(100), stored to a __DATA global and read back
    mov     x0, #100
    bl      _sum_to
    adrp    x8, _result@PAGE
    add     x8, x8, _result@PAGEOFF
    str     x0, [x8]

    // write(1, msg, len)
    mov     x0, #1
    adrp    x1, _msg@PAGE
    add     x1, x1, _msg@PAGEOFF
    mov     x2, #41             // strlen(_msg); Mach-O can't fix up a label diff here
    mov     x16, #4
    svc     #0x80

    // exit(result & 0xff)
    adrp    x8, _result@PAGE
    add     x8, x8, _result@PAGEOFF
    ldr     x0, [x8]
    and     x0, x0, #0xff
    mov     x16, #1
    svc     #0x80
    brk     #1

    .p2align 2
_sum_to:                        // x0 = n -> x0 = 1 + 2 + ... + n
    mov     x1, #0
1:  add     x1, x1, x0
    subs    x0, x0, #1
    b.ne    1b
    mov     x0, x1
    ret

    .section __TEXT,__cstring,cstring_literals
_msg:
    .asciz  "Hello from arm64 Mach-O on dynarmic A64!\n"

    .section __DATA,__data
    .p2align 3
_result:
    .quad   0
