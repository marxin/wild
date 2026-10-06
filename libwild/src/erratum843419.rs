//! Erratum 843419: A load or store might access an incorrect address.
//! Software Developers Errata Notice: https://documentation-service.arm.com/static/5fa29fddb209f547eebd361d
//!
//! Description:
//! When executing in AArch64 state, a load or store instruction which uses the result of an ADRP instruction as a base
//! register, or which uses a base register written by an instruction immediately after an ADRP to the same register, might
//! access an incorrect address.
//!
//! Workaround: Prevent affected sequences from crossing a 4 KiB page boundary by keeping
//! the ADRP away from page offsets 0xFF8 and 0xFFC.
//!
//! Variant 2 is intentionally excluded because it involves a dead ADRP instruction:
//! where the following instruction overwrites its destination register.

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
            // Ldr is a subset of LdrStr, so decode it first.
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
                // The target register (rt) is not needed here.
                rn: (insn >> 5) & REGISTER_MASK,
            }
        } else if Self::is_branch(insn) {
            Self::Branch
        } else {
            Self::Unrecognized
        }
    }

    /// Returns true if the instruction sequence begins with an ADRP that could trigger
    /// erratum 843419.
    fn starts_with_erratum_843419(insns: &[ArmInsn]) -> bool {
        if insns.len() < 3 {
            return false;
        }

        // 1) ADRP
        let ArmInsn::Adrp { rd: register } = insns[0] else {
            return false;
        };

        // 2) The next instruction must not overwrite the ADRP destination register.
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

/// The erratum depends on the low 12 bits of the instruction address.
const ERRATUM_PAGE_SIZE: usize = 4096;
/// The number of instruction positions in one erratum page.
const ERRATUM_INSN_OFFSETS: usize = ERRATUM_PAGE_SIZE / 4;

#[derive(Debug)]
pub(crate) struct ErratumMask {
    // Section start offsets within a page, in instruction units, that could trigger the erratum.
    pub(crate) mask: FixedBitSet,
    // Section alignment in bytes.
    pub(crate) alignment: usize,
    // Maximum padding in bytes needed to make any valid section placement safe.
    pub(crate) maximal_padding: usize,
}

fn erratum_mask_from_offset(
    erratum_offsets: &[usize],
    section_alignment: u64,
) -> Result<Option<ErratumMask>> {
    let section_alignment = section_alignment as usize;
    if erratum_offsets.is_empty() {
        return Ok(None);
    }

    // Set a bit for each section start offset that places an affected ADRP at an unsafe page offset.
    let mut mask = FixedBitSet::with_capacity(ERRATUM_INSN_OFFSETS);
    for section_start in 0..ERRATUM_INSN_OFFSETS {
        if erratum_offsets.iter().any(|offset| {
            let i = (*offset + section_start) % ERRATUM_INSN_OFFSETS;
            i == 0xff8 / 4 || i == 0xffc / 4
        }) {
            mask.put(section_start);
        }
    }

    ensure!(
        // Alignments of at least 4096 place the section at a page boundary, which must be safe.
        section_alignment < ERRATUM_PAGE_SIZE || !mask[0],
        "cannot fix erratum for a section with large alignment: {section_alignment}"
    );

    // For each unsafe, aligned section start offset, find the smallest alignment-multiple
    // shift that moves all affected ADRP instructions away from unsafe page offsets.
    ensure!(
        section_alignment >= 4,
        "unexpected small alignment: {section_alignment}"
    );
    let alignment_in_insns = section_alignment / 4;
    let necessary_shifts = mask
        .ones()
        // Consider only start offsets that satisfy the section alignment.
        .filter(|i| i % alignment_in_insns == 0)
        .map(|i| {
            // Try each multiple of the alignment within a page until a safe placement is found.
            (0..(ERRATUM_INSN_OFFSETS / alignment_in_insns))
                .map(|step| step * alignment_in_insns)
                .find(|step| !mask[(i + *step) % ERRATUM_INSN_OFFSETS])
                .ok_or_else(|| error!("Cannot find a valid offset for an erratum"))
        })
        .collect::<Result<Vec<_>>>()?;
    let maximal_padding = 4 * necessary_shifts.into_iter().max().unwrap_or(0);

    Ok(Some(ErratumMask {
        alignment: section_alignment,
        mask,
        maximal_padding,
    }))
}

pub(crate) fn erratum_mask(data: &[u8], section_alignment: u64) -> Result<Option<ErratumMask>> {
    debug_assert!(section_alignment.is_power_of_two());
    let erratum_offsets = erratum_843419_offsets(data)
        .into_iter()
        .map(|offset| (offset % ERRATUM_PAGE_SIZE) / 4)
        .unique()
        .collect_vec();
    erratum_mask_from_offset(&erratum_offsets, section_alignment)
}

#[test]
fn test_erratum_mask_from_offset() {
    // Offsets are in instruction units; alignment and padding are in bytes.
    let cases: &[(&[usize], u64, Option<usize>)] = &[
        (&[], 4, None),
        (&[], 4096, None),
        (&[0], 4, Some(8)),
        (&[0], 8, Some(8)),
        (&[0], 16, Some(0)),
        (&[1], 4, Some(8)),
        (&[1022], 4, Some(8)),
        (&[1022], 8, Some(8)),
        (&[1022], 16, Some(16)),
        (&[1023], 4, Some(8)),
        (&[1023], 8, Some(8)),
        (&[0, 2], 4, Some(16)),
        (&[0, 2, 4], 4, Some(24)),
        (&[0, 2, 4, 6], 4, Some(32)),
        (&[0, 0], 4, Some(8)),
        (&[1024], 4, Some(8)),
        (&[0], 4096, Some(0)),
        (&[0], 8192, Some(0)),
    ];

    for &(offsets, alignment, expected) in cases {
        assert_eq!(
            erratum_mask_from_offset(offsets, alignment)
                .unwrap()
                .map(|mask| mask.maximal_padding),
            expected,
            "offsets={offsets:?}, alignment={alignment}"
        );
    }

    let error_cases: &[(&[usize], u64)] = &[
        (&[0], 0),
        (&[0], 1),
        (&[0], 2),
        (&[1022], 4096),
        (&[1023], 8192),
        (&[510, 1022], 2048),
    ];
    for &(offsets, alignment) in error_cases {
        assert!(
            erratum_mask_from_offset(offsets, alignment).is_err(),
            "offsets={offsets:?}, alignment={alignment}"
        );
    }
}
