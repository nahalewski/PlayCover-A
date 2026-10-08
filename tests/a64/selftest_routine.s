// EXPERIMENTAL A64 self-test routine (GNU as syntax).
// Assembled with: dev-scripts/build-a64-test-payloads.sh
//
// In:  x0 = n, x1 = address of a 64-byte scratch buffer
// Out: x0 = (sum of 1..n) + (0xdead << 48)
//      [x1+0]  = sum
//      [x1+32] = sum, [x1+40] = 0xdead << 48   (copied via a 128-bit q-reg)
// Ends with `svc #0x80`.
    .text
    .globl selftest_routine
selftest_routine:
    mov     x2, #0
1:  add     x2, x2, x0
    subs    x0, x0, #1
    b.ne    1b
    str     x2, [x1]
    ldr     x3, [x1]
    movz    x4, #0xdead, lsl #48
    add     x0, x3, x4
    stp     x3, x4, [x1, #16]
    ldr     q0, [x1, #16]
    str     q0, [x1, #32]
    svc     #0x80
