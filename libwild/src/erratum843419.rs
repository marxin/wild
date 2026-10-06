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

use fixedbitset::FixedBitSet;
use itertools::Itertools;

use crate::Result;
use crate::ensure;
use crate::error;

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
fn erratum_843419_offsets(data: &[u8]) -> Vec<usize> {
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

/// Erratum is affected by low 12 bits
const ERRATUM_PAGE_SIZE: usize = 4096;
/// The number instruction position to consider for the erratume.
const ERRATUM_INSN_OFFSETS: usize = ERRATUM_PAGE_SIZE / 4;

#[derive(Debug)]
pub(crate) struct ErratumMask {
    // Erratum page offsets (expressed in offsets in instructions) that will trigger the erratum.
    pub(crate) mask: FixedBitSet,
    // Section alignment (in number of instructions)
    pub(crate) alignment: usize,
    // A necessary padding to move all errata from any given offsets in a final binary (in bytes)
    pub(crate) maximal_padding: usize,
}

pub(crate) fn erratum_mask(data: &[u8], section_alignment: u64) -> Result<Option<ErratumMask>> {
    let section_alignment = section_alignment as usize;
    debug_assert!(section_alignment.is_multiple_of(2));
    let erratum_offsets = erratum_843419_offsets(data)
        .into_iter()
        .map(|offset| (offset % ERRATUM_PAGE_SIZE) / 4)
        .unique()
        .collect_vec();

    if erratum_offsets.is_empty() {
        return Ok(None);
    }
    ensure!(
        section_alignment < ERRATUM_PAGE_SIZE,
        "cannot fix erratum for a section with large alignment: {section_alignment}"
    );

    // Mask has bit set to true if that offset will put any of the erratum offests into the wrong position.
    let mut mask = FixedBitSet::with_capacity(ERRATUM_INSN_OFFSETS);
    for section_start in 0..ERRATUM_INSN_OFFSETS {
        if erratum_offsets.iter().any(|offset| {
            let i = (*offset + section_start) % ERRATUM_INSN_OFFSETS;
            i == 0xff8 / 4 || i == 0xffc / 4
        }) {
            mask.put(section_start);
        }
    }

    // Now for each bit set (bad page offset), calculate how many multiples of an alignment will be needed to fix it
    // so each erratum will be out of the problematic offests.
    ensure!(
        section_alignment >= 4,
        "unexpected small alignment: {section_alignment}"
    );
    let alignment_in_insns = section_alignment / 4;
    let necessary_shifts = mask
        .ones()
        .map(|i| {
            // Just try all the possible multiples of a the alignment and check if we reach
            // a configuration that is safe.
            (0..(ERRATUM_INSN_OFFSETS / alignment_in_insns))
                .map(|step| step * alignment_in_insns)
                .find(|step| !mask[(i + *step) % ERRATUM_INSN_OFFSETS])
                .ok_or_else(|| error!("Cannot find a valid offset for an erratum"))
        })
        .collect::<Result<Vec<_>>>()?;
    let maximal_padding = 4 * necessary_shifts.into_iter().max().unwrap();

    Ok(Some(ErratumMask {
        alignment: section_alignment,
        mask,
        maximal_padding,
    }))
}
