# kernel/sched/switch.S
# User-space context switch: the two halves of every kernel<->user
# crossing. LightOS v1 schedules by switching *trap frames* (the kernel
# itself is non-preemptible and runs on one kernel stack), so these two
# routines ARE the context switch.
#
# Layout contract with trap/context.rs: register xN at byte offset
# N*8, sepc at 256, sstatus at 264, kernel_sp at 272.

.option norvc
.section .text

# Trap entry for traps taken from U-mode. sscratch holds the current
# process's TrapFrame pointer while in user mode.
.global __user_vec
.align 4
__user_vec:
    # x31 <-> sscratch: x31 = &TrapFrame, sscratch = user x31.
    csrrw   x31, sscratch, x31

    sd      x1,   8(x31)
    sd      x2,  16(x31)
    sd      x3,  24(x31)
    sd      x4,  32(x31)
    sd      x5,  40(x31)
    sd      x6,  48(x31)
    sd      x7,  56(x31)
    sd      x8,  64(x31)
    sd      x9,  72(x31)
    sd      x10, 80(x31)
    sd      x11, 88(x31)
    sd      x12, 96(x31)
    sd      x13, 104(x31)
    sd      x14, 112(x31)
    sd      x15, 120(x31)
    sd      x16, 128(x31)
    sd      x17, 136(x31)
    sd      x18, 144(x31)
    sd      x19, 152(x31)
    sd      x20, 160(x31)
    sd      x21, 168(x31)
    sd      x22, 176(x31)
    sd      x23, 184(x31)
    sd      x24, 192(x31)
    sd      x25, 200(x31)
    sd      x26, 208(x31)
    sd      x27, 216(x31)
    sd      x28, 224(x31)
    sd      x29, 232(x31)
    sd      x30, 240(x31)

    # Recover the user's x31 from sscratch and park the frame pointer
    # back in sscratch.
    csrr    t0, sscratch
    sd      t0, 248(x31)
    csrw    sscratch, x31

    csrr    t0, sepc
    sd      t0, 256(x31)
    csrr    t0, sstatus
    sd      t0, 264(x31)

    # Kernel traps go to the kernel vector while we're inside.
    la      t0, __trap_vector
    csrw    stvec, t0

    mv      a0, x31
    ld      sp, 272(x31)         # fresh kernel stack
    call    usertrap             # -> ! (never returns here)

# Enter user mode: __user_ret(a0 = &TrapFrame, a1 = satp). Caller has
# already staged sepc/sstatus values in the frame and set SPP=U.
.global __user_ret
__user_ret:
    # Switch to the process address space (kernel is mapped in every
    # process root, so execution continues seamlessly).
    sfence.vma  zero, zero
    csrw    satp, a1
    sfence.vma  zero, zero

    # Traps from user mode land in __user_vec.
    la      t0, __user_vec
    csrw    stvec, t0
    csrw    sscratch, a0

    ld      t0, 256(a0)
    csrw    sepc, t0
    ld      t0, 264(a0)
    csrw    sstatus, t0

    ld      x1,   8(a0)
    ld      x2,  16(a0)
    ld      x3,  24(a0)
    ld      x4,  32(a0)
    ld      x5,  40(a0)
    ld      x6,  48(a0)
    ld      x7,  56(a0)
    ld      x8,  64(a0)
    ld      x9,  72(a0)
    # x10 (a0) restored last — it's our frame pointer.
    ld      x11, 88(a0)
    ld      x12, 96(a0)
    ld      x13, 104(a0)
    ld      x14, 112(a0)
    ld      x15, 120(a0)
    ld      x16, 128(a0)
    ld      x17, 136(a0)
    ld      x18, 144(a0)
    ld      x19, 152(a0)
    ld      x20, 160(a0)
    ld      x21, 168(a0)
    ld      x22, 176(a0)
    ld      x23, 184(a0)
    ld      x24, 192(a0)
    ld      x25, 200(a0)
    ld      x26, 208(a0)
    ld      x27, 216(a0)
    ld      x28, 224(a0)
    ld      x29, 232(a0)
    ld      x30, 240(a0)
    ld      x31, 248(a0)
    ld      x10, 80(a0)

    sret
