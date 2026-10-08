.text
.globl _main
.p2align 2
_main:
    cmp x0, #1 // argc
    b.ne failed
    cbz x1, failed
    cbz x2, failed
    cbz x3, failed
    ldr x4, [x1]
    cbz x4, failed
    ldrb w5, [x4]
    cmp w5, #47 // argv[0] starts with '/'
    b.ne failed
    ldr x4, [x1, #8]
    cbnz x4, failed
    ldr x4, [x2]
    cbnz x4, failed
    ldr x4, [x3]
    cbz x4, failed
    ldrb w5, [x4]
    cmp w5, #101 // apple[0] starts with 'executable_path='
    b.ne failed
    ldr x4, [x3, #8]
    cbnz x4, failed
    mov x4, sp
    tst x4, #15
    b.ne failed
    mov x16, #20 // getpid
    svc #0x80
    b.cs failed
    cbz x0, failed
    mov x16, #372 // thread_selfid
    svc #0x80
    b.cs failed
    cbz x0, failed
    mov x21, x0
    svc #0x80
    cmp x0, x21
    b.ne failed

    mov x0, #99 // invalid descriptor, even for zero-byte write
    mov x1, #0
    mov x2, #0
    mov x16, #4
    svc #0x80
    b.cc failed
    cmp x0, #9 // EBADF
    b.ne failed

    mov x0, #0 // non-fixed address hint
    mov x1, #16384
    mov x2, #3 // VM_PROT_READ | VM_PROT_WRITE
    mov x3, #0x1002 // MAP_ANON | MAP_PRIVATE
    mov x4, #-1 // anonymous descriptor
    mov x5, #0 // offset
    mov x16, #197
    svc #0x80
    b.cs failed
    cbz x0, failed
    ldrb w1, [x0]
    mov x19, x0
    cbnz w1, failed
    mov w1, #42
    strb w1, [x0]
    ldrb w0, [x0]
    cmp w0, #42
    b.ne failed
    // Keep the anonymous mapping for all validated time outputs.
    // x0 was overwritten above by ldrb, so use the saved mapping in x19.
time_checks:
    cmp xzr, xzr // carry set; negative Mach traps must preserve it
    mov w16, #-3 // low32 signed selector, zero-extended in x16
    svc #0x80
    b.cc failed
    mov x20, x0
    mov x16, #-3
    svc #0x80
    b.cc failed
    cmp x0, x20
    b.lo failed

    mov x0, x19
    mov x16, #-89 // mach_timebase_info
    cmp xzr, xzr
    svc #0x80
    b.cc failed
    cbnz x0, failed
    ldr w1, [x19]
    cmp w1, #1
    b.ne failed
    ldr w1, [x19, #4]
    cmp w1, #1
    b.ne failed

    mov x0, x19
    add x1, x19, #16
    add x2, x19, #24
    mov x16, #116 // gettimeofday(timeval, timezone, absolute ticks)
    svc #0x80
    b.cs failed
    cbnz x0, failed
    ldr x1, [x19]
    cbz x1, failed
    ldr w1, [x19, #8]
    mov w2, #0x4240
    movk w2, #0xf, lsl #16 // 1,000,000 microseconds
    cmp w1, w2
    b.hs failed
    ldr w1, [x19, #12] // timeval padding zero
    cbnz w1, failed
    ldr x1, [x19, #16] // virtual UTC timezone
    cbnz x1, failed
    ldr x1, [x19, #24]
    cmp x1, x20
    b.lo failed

    // A late invalid output cannot change an earlier valid output.
    add x0, x19, #64
    mov x3, #0x1234
    str x3, [x0]
    add x1, x19, #80
    add x2, x19, #4, lsl #12 // unmapped end of the 16 KiB region
    mov x16, #116
    svc #0x80
    b.cc failed
    cmp x0, #14
    b.ne failed
    ldr x1, [x19, #64]
    cmp x1, x3
    b.ne failed
    mov x0, #42
    mov x16, #1
    svc #0x80
    brk #0
failed:
    mov x0, #101
    mov x16, #1
    svc #0x80
    brk #0
