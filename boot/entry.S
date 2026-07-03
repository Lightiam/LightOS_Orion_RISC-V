# boot/entry.S
# RISC-V RV64GC only. QEMU `-bios none` starts every hart here at
# 0x8000_0000 in machine mode with a0 = hartid, a1 = DTB pointer.
#
# Responsibilities:
#   1. Give each hart its own boot stack.
#   2. Hart 0: clear .bss, then call mstart() (Rust, machine mode),
#      which drops to supervisor mode and never returns.
#   3. Other harts: park in wfi until SMP bring-up (post-v1).

.option norvc
.section .text.entry
.global _start
_start:
    # Load global pointer for gp-relative addressing (relaxation off
    # around the load itself, as gp is not yet valid).
.option push
.option norelax
    la      gp, __global_pointer$
.option pop

    # sp = _stack_start + (mhartid + 1) * BOOT_STACK_SIZE
    csrr    t0, mhartid
    addi    t1, t0, 1
    li      t2, 0x10000          # 64 KiB per hart, matches linker.ld
    mul     t1, t1, t2
    la      sp, _stack_start
    add     sp, sp, t1

    # Park secondary harts until SMP bring-up.
    bnez    t0, park

    # Clear .bss (hart 0 only, before any Rust code runs).
    la      t0, _bss_start
    la      t1, _bss_end
1:  bgeu    t0, t1, 2f
    sd      zero, 0(t0)
    addi    t0, t0, 8
    j       1b
2:
    # mstart(hartid: a0, dtb: a1) — machine-mode Rust init; performs
    # the mret into supervisor-mode kinit() and never returns.
    call    mstart

park:
    wfi
    j       park
