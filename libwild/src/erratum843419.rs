//! Erratum 843419: A load or store might access an incorrect address
//! Software Developers Errata Notice link: https://documentation-service.arm.com/static/5fa29fddb209f547eebd361d
//!
//! Description:
//! When executing in AArch64 state, a load or store instruction which uses the result of an ADRP instruction as a base
//! register, or which uses a base register written by an instruction immediately after an ADRP to the same register, might
//! access an incorrect address.
//!
//! Workaround: Ensuring the ADRP and the load/store is in a diffeent page (ADRP cannot be placed at address ending with 0xFF8 or 0xFFC).
//!
//! Note the Variant 2 is intentionally excluded as it depends on a dead instruction
//! (where register of ADRP insn is overwritten by the following one).

use itertools::Itertools;

const ADRP_MARK: u32 = 0x9f00_0000;
const ADRP_OPCODE: u32 = 0x9000_0000;
// LDR (unsigned offset)
const LDR_UNSIGNED_MASK: u32 = 0xffc0_0000;
const LDR_UNSIGNED_OPCODE: u32 = 0xf940_0000;
// ADD (immediate)
const ADD_IMM_MASK: u32 = 0xffc0_0000;
const ADD_IMM_OPCODE: u32 = 0x9100_0000;
// Load/store register (unsigned immediate) (ARM Manual category of instructions)
const LDR_STR_UNSIGNED_MASK: u32 = 0x3b00_0000;
const LDR_STR_UNSIGNED_OPCODE: u32 = 0x3900_0000;

const REGISTER_MASK: u32 = (1 << 5) - 1;

#[derive(Debug)]
enum ArmInsn {
    Adrp { rd: u32 },
    Branch,
    Ldr { rt: u32, rn: u32 },
    Add { rd: u32 },
    LdrStr { rn: u32 },
    Unrecognized,
}

impl ArmInsn {
    fn is_final_load_store_imm(insn: &ArmInsn, register: u32) -> bool {
        match insn {
            Self::LdrStr { rn, .. } if *rn == register => true,
            Self::Ldr { rn, .. } if *rn == register => true,
            _ => false,
        }
    }

    fn is_branch(insn: u32) -> bool {
        // B, BL
        (insn & 0x7c000000) == 0x14000000
           // B.cond and BC.cond
           || (insn & 0xff000000) == 0x54000000
           // CBZ, CBNZ
           || (insn & 0x7e000000) == 0x34000000
           // TBZ, TBNZ
           || (insn & 0x7e000000) == 0x36000000
           // BR, BLR, RET and authenticated/register control transfers
           || (insn & 0xfe000000) == 0xd6000000
    }

    fn from_opcode(insn: u32) -> Self {
        if insn & ADRP_MARK == ADRP_OPCODE {
            Self::Adrp {
                rd: insn & REGISTER_MASK,
            }
        } else if insn & LDR_UNSIGNED_MASK == LDR_UNSIGNED_OPCODE {
            // Note Ldr is a actually a part of LdrStr, parse it earlier.
            Self::Ldr {
                rt: insn & REGISTER_MASK,
                rn: (insn >> 5) & REGISTER_MASK,
            }
        } else if insn & ADD_IMM_MASK == ADD_IMM_OPCODE {
            Self::Add {
                rd: insn & REGISTER_MASK,
            }
        } else if insn & LDR_STR_UNSIGNED_MASK == LDR_STR_UNSIGNED_OPCODE {
            Self::LdrStr {
                // rt is not used
                rn: (insn >> 5) & REGISTER_MASK,
            }
        } else if Self::is_branch(insn) {
            Self::Branch
        } else {
            Self::Unrecognized
        }
    }

    /// Return true if the sequence of instructions starts with ADRP that is subject
    /// to the erratum. Otherwise, return false.
    fn starts_with_erratum_843419(insns: &[ArmInsn]) -> bool {
        if insns.len() < 3 {
            return false;
        }

        // 1) ADRP
        let ArmInsn::Adrp { rd: register } = insns[0] else {
            return false;
        };

        // 2) A load or store instruction:
        // ...
        // This must not write to Rn.
        match insns[1] {
            Self::Add { .. } | Self::Adrp { .. } | Self::Branch => return false,
            ArmInsn::Ldr { rt, .. } if rt == register => return false,
            _ => {}
        }

        // 3) Variant A (optional 3rd instruction)
        if insns.len() >= 4 {
            // This cannot be a branch.
            // This cannot write Rn.
            match insns[2] {
                Self::Branch => {}
                Self::Add { rd, .. } if rd == register => {}
                Self::Ldr { rt, .. } if rt == register => {}
                ArmInsn::Adrp { rd } if rd == register => {}
                _ => {
                    // 4) Load/store register (unsigned immediate)" encoding class, using Rn as the base address register.
                    if Self::is_final_load_store_imm(&insns[3], register) {
                        return true;
                    }
                }
            }
        }

        // 3) Variant B
        Self::is_final_load_store_imm(&insns[2], register)
    }
}

/// Returns byte offsets of all potential erratum sequences in the instruction stream.
pub(crate) fn erratum_843419_offsets(data: &[u8]) -> Vec<usize> {
    let insns = data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| ArmInsn::from_opcode(u32::from_le_bytes(*bytes)))
        .collect_vec();

    (0..insns.len())
        .filter(|&index| ArmInsn::starts_with_erratum_843419(&insns[index..]))
        .map(|index| index * size_of::<u32>())
        .collect()
}
